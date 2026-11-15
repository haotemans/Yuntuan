use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use yuantuan_core::{
    db::{self, params, SqliteExt},
    llm::LlmGateway,
    memory::{self, nightly, refresh},
};

struct Rig {
    dir: PathBuf,
    path: PathBuf,
    gw: Arc<LlmGateway>,
    slot: yuantuan_core::bot::SharedLlm,
    calls: Arc<Mutex<Vec<Value>>>,
    peak: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
impl Rig {
    async fn new(output: String) -> Self {
        Self::outputs(vec![output]).await
    }
    async fn outputs(outputs: Vec<String>) -> Self {
        let outputs = Arc::new(Mutex::new(std::collections::VecDeque::from(outputs)));
        let dir = std::env::temp_dir().join(format!(
            "yt-profile-refresh-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        db::migrate(&mut db::connect(&path).await.unwrap())
            .await
            .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let peak_seen = peak.clone();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let outputs = outputs.clone();
                let seen = seen.clone();
                let live = live.clone();
                let peak = peak_seen.clone();
                async move {
                    peak.fetch_max(live.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    seen.lock().unwrap().push(body);
                    live.fetch_sub(1, Ordering::SeqCst);
                    let output = {
                        let mut values = outputs.lock().unwrap();
                        if values.len() > 1 {
                            values.pop_front().unwrap()
                        } else {
                            values.front().unwrap().clone()
                        }
                    };
                    axum::Json(
                        json!({"choices":[{"message":{"content":output},"finish_reason":"stop"}]}),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let config = dir.join("providers.toml");
        std::fs::write(&config,format!("[provider.mock]\nbase_url=\"http://{address}\"\n[roles]\ndecision={{provider=\"mock\",model=\"test\"}}\n")).unwrap();
        let gw = Arc::new(LlmGateway::load(&config).unwrap());
        let slot = Arc::new(std::sync::RwLock::new(Some(gw.clone())));
        Self {
            dir,
            path,
            gw,
            slot,
            calls,
            peak,
            server,
        }
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn refresh_reads_the_oldest_pending_prefix_and_rejects_unloaded_evidence() {
    let r = Rig::new(json!({"updates":[
        {"field":"interests","value":"喜欢玩明日方舟","evidence_msg_id":1,"evidence_quote":"我喜欢玩明日方舟"},
        {"field":"preferred_name","value":"不应写入","evidence_msg_id":159,"evidence_quote":"我的密码不应该入模"}
    ]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute(
        "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','A',1,170)",
        params![],
    )
    .await
    .unwrap();
    for id in 1..=170 {
        let text = match id {
            1 => "我喜欢玩明日方舟",
            159 => "我的密码不应该入模",
            _ => "只是一条普通的讨论消息",
        };
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(?1,'g','group','p_a',?2,?1)",params![id,text]).await.unwrap();
    }
    let result = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(result.status, "updated");
    assert_eq!(result.inspected_messages, 159);
    assert_eq!(
        memory::profile(&mut conn, "p_a", i64::MAX)
            .await
            .unwrap()
            .len(),
        1
    );
    let processed: i64 = sqlx::query_scalar(
        "SELECT cursor FROM memory_progress WHERE stage='portrait' AND owner_id='p_a'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(processed, 160);
    let calls = r.calls.lock().unwrap();
    let input: Value =
        serde_json::from_str(calls[0]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert!(!input.to_string().contains("我的密码不应该入模"));
    assert!(input["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["msg_id"].as_i64().unwrap() <= 160));
}

#[tokio::test]
async fn repair_uses_own_group_and_private_messages_preserves_current_and_group_history() {
    let r=Rig::new(json!({"updates":[
        {"field":"technical_preferences","value":"长期使用 Rust","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"},
        {"field":"interests","value":"喜欢明日方舟","evidence_msg_id":2,"evidence_quote":"我喜欢玩明日方舟"},
        {"field":"preferred_name","value":"别人的称呼","evidence_msg_id":3,"evidence_quote":"请叫我小明"},
        {"field":"communication_style","value":"伪造资料","evidence_msg_id":1,"evidence_quote":"不存在的原话"},
        {"field":"ongoing_projects","value":"旧的项目描述","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"}
    ]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','同名',1,5),('p_b','同名',1,3),('self','云团',1,4);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
        (1,'g1','group','p_a','我长期使用 Rust',1),(2,'g1','private','p_a','我喜欢玩明日方舟',2),
        (3,'g2','group','p_b','请叫我小明',3),(4,'g2','group','self','机器人旧回答',4),
        (5,'g2','group','p_a','我在开发一个游戏项目',5);
        INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at)
        VALUES('p_a','ongoing_projects','开发游戏',5,'我在开发一个游戏项目',5);
        INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at)
        VALUES('chat','g1','群聊记忆保留','consolidation',1,1),('person','p_a','旧摘要不充当画像证据','consolidation',1,1);
        INSERT INTO summaries(owner_type,owner_id,period,date,summary,created_at) VALUES('chat','g1','daily','2026-11-04','群聊每日摘要保留',1);").await.unwrap();
    let first = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(first.status, "updated");
    assert_eq!(first.updated_fields, 1);
    // Scope batches cannot expose private facts to a group-derived synthesis.
    conn.execute_batch("UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:p_a'").await.unwrap();
    let private_pass = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(private_pass.updated_fields, 1);
    let facts = memory::profile(&mut conn, "p_a", i64::MAX).await.unwrap();
    assert_eq!(facts.len(), 3);
    assert!(facts
        .iter()
        .any(|f| f.field == "interests" && f.source_msg_id == 2));
    assert!(facts
        .iter()
        .any(|f| f.field == "ongoing_projects" && f.content == "开发游戏"));
    assert!(memory::profile(&mut conn, "p_b", i64::MAX)
        .await
        .unwrap()
        .is_empty());
    let second = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(second.status, "unchanged");
    let calls = r.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let input: Value =
        serde_json::from_str(calls[0]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["messages"].as_array().unwrap().len(), 2);
    assert!(!input["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["chat_type"] == "private"));
    assert!(!input.to_string().contains("旧摘要不充当画像证据"));
    drop(calls);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM long_memories")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT summary FROM summaries")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        "群聊每日摘要保留"
    );
}

#[tokio::test]
async fn nightly_quota_survives_restarts_and_empty_results_rotate_to_other_people() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    for id in 1..=10 {
        let pid = format!("p_{id}");
        conn.execute(
            "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES(?1,?1,1,1)",
            params![pid],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group',?1,'普通寒暄不应凭空生成性格',1)",params![pid]).await.unwrap();
    }
    let options = night_options("2026-11-06", 4);
    nightly::run(&r.path, &r.slot, &options).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 4);
    assert_eq!(r.peak.load(Ordering::SeqCst), 1);
    nightly::run(&r.path, &r.slot, &options).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 4);
    nightly::run(&r.path, &r.slot, &night_options("2026-11-07", 6))
        .await
        .unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 10);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM person_profile_facts")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn malformed_results_leave_profiles_intact_and_rapid_retries_do_not_call_model_again() {
    let r = Rig::new("not json".into()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','A',1,1);
        INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','p_a','我长期使用 Rust',1);
        INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at) VALUES('p_a','technical_preferences','Rust',1,'我长期使用 Rust',1);").await.unwrap();
    assert!(refresh::person(&r.path, &r.gw, "p_a", true).await.is_err());
    let state: String =
        sqlx::query_scalar("SELECT value FROM state_kv WHERE key='profile_refresh:v1:p_a'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&state).unwrap()["status"],
        "failed"
    );
    assert_eq!(
        refresh::person(&r.path, &r.gw, "p_a", true)
            .await
            .unwrap()
            .status,
        "unchanged"
    );
    assert_eq!(r.calls.lock().unwrap().len(), 2);
    assert_eq!(
        memory::profile(&mut conn, "p_a", i64::MAX).await.unwrap()[0].content,
        "Rust"
    );
}

#[tokio::test]
async fn nightly_updates_existing_portraits_and_requires_new_messages() {
    let r=Rig::new(json!({"updates":[{"field":"technical_preferences","value":"主要使用 Python","evidence_msg_id":2,"evidence_quote":"我现在主要使用 Python"}]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','A',1,2);
      INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
      (1,'g1','group','p_a','我长期使用 Rust',1),(2,'private','private','p_a','我现在主要使用 Python',CAST(strftime('%s','now') AS INTEGER));
      INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at)
      VALUES('p_a','technical_preferences','Rust',1,'我长期使用 Rust',1);").await.unwrap();
    conn.execute_batch("INSERT INTO memory_progress(stage,owner_id,chat_type,cursor,baseline) VALUES('portrait','p_a','',1,1)").await.unwrap();
    nightly::run(&r.path, &r.slot, &night_options("2026-11-06", 10))
        .await
        .unwrap();
    assert_eq!(
        memory::profile(&mut conn, "p_a", i64::MAX).await.unwrap()[0].content,
        "主要使用 Python"
    );
    let state: String =
        sqlx::query_scalar("SELECT value FROM state_kv WHERE key='profile_refresh:v1:p_a'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["status"], "updated");
    assert_eq!(state["cutoff"], 2);
    assert!(state["completed_at"].as_i64().unwrap() > 0);
    // A fresh pass without new text never calls the model, even after the cooldown.
    conn.execute_batch("UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:p_a'")
        .await
        .unwrap();
    nightly::run(&r.path, &r.slot, &night_options("2026-11-07", 10))
        .await
        .unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_nightly_attempts_consume_the_daily_quota() {
    let r = Rig::new("not json".into()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    for id in 1..=3 {
        let pid = format!("p_{id}");
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES(?1,1,1)",
            params![pid],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group',?1,'我长期使用 Rust',1)",params![pid]).await.unwrap();
    }
    let options = night_options("2026-11-06", 2);
    nightly::run(&r.path, &r.slot, &options).await.unwrap();
    nightly::run(&r.path, &r.slot, &options).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 4);
    let used:i64=sqlx::query_scalar("SELECT json_extract(value,'$.used') FROM state_kv WHERE key='profile_nightly:v1:2026-11-06'").fetch_one(&mut *conn).await.unwrap();
    assert_eq!(used, 2);
}

fn night_options(date: &str, limit: usize) -> nightly::Options {
    nightly::Options {
        date: date.into(),
        deadline: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600,
        limit,
        interval_secs: 0,
    }
}

#[tokio::test]
async fn recent_evidence_updates_now_without_skipping_historical_gaps() {
    let r=Rig::new(json!({"updates":[{"field":"ongoing_projects","value":"在做聊天机器人","evidence_msg_id":501,"evidence_quote":"我正在开发聊天机器人"}]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1)")
        .await
        .unwrap();
    for id in 1..=501 {
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(?1,'g','group','a',?2,?3)",
            params![id,if id==501 {"我正在开发聊天机器人"} else {"历史普通讨论没有个人信息"},if id==501 {memory::processing::now()} else {id}]).await.unwrap();
    }
    let result = refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    assert_eq!(result.status, "updated");
    assert_eq!(result.inspected_messages, 160);
    assert_eq!(
        memory::processing::cursor(&mut conn, &memory::processing::Scope::portrait("a"))
            .await
            .unwrap(),
        159
    );
    let covered: Vec<i64> =
        sqlx::query_scalar("SELECT msg_id FROM portrait_coverage ORDER BY msg_id")
            .fetch_all(&mut *conn)
            .await
            .unwrap();
    assert!(covered.contains(&501));
    assert!(!covered.contains(&160));
    let pending:i64=sqlx::query_scalar("SELECT COUNT(*) FROM messages m WHERE NOT EXISTS(SELECT 1 FROM portrait_coverage c WHERE c.msg_id=m.msg_id)").fetch_one(&mut *conn).await.unwrap();
    assert_eq!(pending, 341);
    let state: String =
        sqlx::query_scalar("SELECT value FROM state_kv WHERE key='profile_refresh:v1:a'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["model"], "test");
    assert!(state["last_success_at"].as_i64().unwrap() > 0);
    assert_eq!(
        memory::profile(&mut conn, "a", i64::MAX).await.unwrap()[0].source_msg_id,
        501
    );
}

#[tokio::test]
async fn recent_character_budget_reserves_backfill_and_migration_preserves_old_memories() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1);
        INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at) VALUES('chat','g','必须保留群记忆','consolidation',1,1);
        DROP TABLE portrait_runs; DROP TABLE portrait_coverage; PRAGMA user_version=18;").await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        19
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT content FROM long_memories")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        "必须保留群记忆"
    );
    for id in 1..=180 {
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(?1,'g','group','a',?2,?3)",
            params![id,"原".repeat(4000),if id<=30 {id} else {memory::processing::now()}]).await.unwrap();
    }
    refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    let ids: Vec<i64> = sqlx::query_scalar("SELECT msg_id FROM portrait_coverage ORDER BY msg_id")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert_eq!(ids.len(), 8);
    assert!(ids.contains(&1) && ids.contains(&2) && ids.contains(&180));
    assert!(!ids.contains(&3));
    assert_eq!(
        memory::processing::cursor(&mut conn, &memory::processing::Scope::portrait("a"))
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn one_format_repair_can_succeed_and_invalid_evidence_never_advances_coverage() {
    let r=Rig::outputs(vec!["bad JSON".into(),json!({"updates":[]}).to_string(),
        json!({"updates":[{"field":"interests","value":"伪造兴趣","evidence_msg_id":2,"evidence_quote":"不存在的原话"}]}).to_string()]).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1);
      INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'g','group','a','只是普通讨论消息',1)").await.unwrap();
    refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    let repairs: i64 = sqlx::query_scalar("SELECT repairs FROM portrait_runs")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(repairs, 1);
    conn.execute_batch("UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:a';
      INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(2,'g','group','a','今天也只是普通讨论',2)").await.unwrap();
    let e = refresh::person(&r.path, &r.gw, "a", true)
        .await
        .err()
        .unwrap();
    assert_eq!(e.to_string(), "invalid_evidence");
    let state: String =
        sqlx::query_scalar("SELECT value FROM state_kv WHERE key='profile_refresh:v1:a'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["error_kind"], "invalid_evidence");
    assert!(state["last_success_at"].as_i64().unwrap() > 0);
    assert_eq!(r.calls.lock().unwrap().len(), 3);
    assert_eq!(
        memory::processing::cursor(&mut conn, &memory::processing::Scope::portrait("a"))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn an_expired_interrupted_batch_is_diagnosed_and_its_unread_sources_can_resume() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1);
       INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','中断前未完成的原始讨论',1);
       INSERT INTO memory_progress(stage,owner_id,chat_type,cursor) VALUES('portrait','a','',0);
       INSERT INTO memory_batches(id,stage,owner_id,chat_type,from_id,end_id,status,started_at,lease_until) VALUES(77,'portrait','a','',0,1,'running',1,2);
       INSERT INTO portrait_runs(batch_id,provider,model,reasoning) VALUES(77,'old','old-model','low');
       INSERT INTO state_kv(key,value,updated_at) VALUES('profile_refresh:v1:a','{\"attempt\":1,\"status\":\"running\"}',1);").await.unwrap();
    refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    let old: (String, String) =
        sqlx::query_as("SELECT status,error_kind FROM portrait_runs WHERE batch_id=77")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(old, ("failed".into(), "interrupted".into()));
    assert_eq!(
        memory::processing::cursor(&mut conn, &memory::processing::Scope::portrait("a"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(r.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn backfill_cannot_replace_newer_semantic_memory_but_fresh_evidence_can() {
    let update = |id, quote, content| {
        json!({"key":"project:bot","kind":"fact","content":content,
        "status":"current","evidence":[{"msg_id":id,"quote":quote}]})
    };
    let r = Rig::outputs(vec![
        json!({"updates":[],"memory_updates":[update(1,"我以前在做旧项目","以前的旧项目")]})
            .to_string(),
        json!({"updates":[],"memory_updates":[update(6,"我后来完成了新项目","新项目已完成")]})
            .to_string(),
    ])
    .await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1);
       INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
       (1,'g','group','a','我以前在做旧项目',1),(5,'g','group','a','我现在在做新项目',5);",
    )
    .await
    .unwrap();
    let newer: memory::semantic::Update =
        serde_json::from_value(update(5, "我现在在做新项目", "正在做新项目")).unwrap();
    memory::semantic::write(&mut conn, "person", "a", 1, 5, &newer)
        .await
        .unwrap();
    let result = refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    assert_eq!(result.status, "no_evidence");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM memory_items")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        1
    );
    conn.execute_batch("UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:a';
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(6,'g','group','a','我后来完成了新项目',6)").await.unwrap();
    assert_eq!(
        refresh::person(&r.path, &r.gw, "a", true)
            .await
            .unwrap()
            .status,
        "updated"
    );
    let current = memory::semantic::current(&mut conn, "person", "a", "g", "group", 6, 6)
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["content"], "新项目已完成");
}

#[tokio::test]
async fn nightly_reloads_binding_after_spacing_and_circuits_repeated_failures() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1);
      INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','普通讨论消息之一',1),('g','group','b','普通讨论消息之二',1)").await.unwrap();
    let options = nightly::Options {
        interval_secs: 1,
        ..night_options("2026-11-08", 2)
    };
    let slot = r.slot.clone();
    let path = r.path.clone();
    let job = tokio::spawn(async move { nightly::run(&path, &slot, &options).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while r.calls.lock().unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let config = r.dir.join("providers.toml");
    std::fs::write(
        &config,
        std::fs::read_to_string(&config)
            .unwrap()
            .replace("model=\"test\"", "model=\"changed\""),
    )
    .unwrap();
    *r.slot.write().unwrap() = Some(Arc::new(LlmGateway::load(&config).unwrap()));
    job.await.unwrap();
    let models: Vec<String> = r
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c["model"].as_str().unwrap().into())
        .collect();
    assert_eq!(models, vec!["test", "changed"]);

    let bad = Rig::new("invalid JSON".into()).await;
    let mut conn = db::connect(&bad.path).await.unwrap();
    for id in 0..5 {
        let pid = format!("p_{id}");
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES(?1,1,1)",
            params![pid],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group',?1,'普通讨论消息没有事实',1)",params![pid]).await.unwrap();
    }
    let options = night_options("2026-11-09", 10);
    nightly::run(&bad.path, &bad.slot, &options).await.unwrap();
    nightly::run(&bad.path, &bad.slot, &options).await.unwrap();
    assert_eq!(bad.calls.lock().unwrap().len(), 6); // Three people, each with at most one format repair.
    let state: String =
        sqlx::query_scalar("SELECT value FROM state_kv WHERE key='profile_nightly:v1:2026-11-09'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["used"], 3);
    assert_eq!(state["failure_streak"], 3);
    assert_eq!(state["active_person"], Value::Null);
}

#[tokio::test]
async fn simultaneous_explicit_refreshes_use_one_model_at_a_time() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1);
      INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','我长期使用 Rust',1),('p','private','b','我长期使用 Python',2);").await.unwrap();
    let (a, b) = tokio::join!(
        refresh::person(&r.path, &r.gw, "a", true),
        refresh::person(&r.path, &r.gw, "b", true)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 2);
    assert_eq!(r.peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn character_budget_never_advances_past_unread_messages() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_a',1,1)")
        .await
        .unwrap();
    for id in 1..=20 {
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(?1,'g','group','p_a',?2,?1)",params![id,"原".repeat(4000)]).await.unwrap();
    }
    for expected in [8, 16, 20] {
        refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
        let processed: i64 = sqlx::query_scalar(
            "SELECT cursor FROM memory_progress WHERE stage='portrait' AND owner_id='p_a'",
        )
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        assert_eq!(processed, expected);
        conn.execute_batch("UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:p_a'").await.unwrap();
    }
    refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn portrait_write_failures_roll_back_versions_and_progress_but_invalid_items_are_skipped() {
    let r = Rig::new(json!({"updates":[
        {"field":"technical_preferences","value":"使用 Rust","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"},
        {"field":"interests","value":"","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"},
        {"field":"preferred_name","value":"小A","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"}
    ]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1);
      INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','我长期使用 Rust',1);
      CREATE TRIGGER fail_portrait BEFORE INSERT ON person_profile_facts WHEN NEW.field='preferred_name'
        BEGIN SELECT RAISE(ABORT,'simulated write failure'); END;").await.unwrap();
    assert!(refresh::person(&r.path, &r.gw, "a", true).await.is_err());
    for table in ["person_profile_facts", "person_profile_versions"] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&mut *conn)
                .await
                .unwrap(),
            0
        );
    }
    let scope = memory::processing::Scope::portrait("a");
    assert_eq!(
        memory::processing::cursor(&mut conn, &scope).await.unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER fail_portrait; UPDATE state_kv SET value=json_set(value,'$.attempt',0) WHERE key='profile_refresh:v1:a';").await.unwrap();
    let result = refresh::person(&r.path, &r.gw, "a", true).await.unwrap();
    assert_eq!(result.updated_fields, 2);
    assert_eq!(
        memory::processing::cursor(&mut conn, &scope).await.unwrap(),
        1
    );
    assert_eq!(r.calls.lock().unwrap().len(), 2);
}
