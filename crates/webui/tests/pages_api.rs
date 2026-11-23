//! 页面后端 API 综合集成测试：真起 serve（临时端口）→ login → 逐端点断言 →
//! /ws 握手收事件（POST /api/config 触发的 ConfigReloaded）。

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use yuantuan_core::db;
use yuantuan_core::db::SqliteExt;
use yuantuan_webui::{serve, Extras};

#[tokio::test]
async fn task_runtime_exposes_versioned_requirements_and_preserves_legacy_tasks() {
    let rig = start().await;
    let mut c = db::connect(&rig.db_path).await.unwrap();
    let anchor: i64 = sqlx::query_scalar(
        "SELECT msg_id FROM messages WHERE sender_pid='p_2001' ORDER BY msg_id LIMIT 1",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    let id = yuantuan_core::agent::create_scoped_task(
        &rig.db_path,
        &yuantuan_core::event::EventBus::default(),
        "555666",
        "group",
        "p_2001",
        Some(anchor),
        "核对资料",
        Default::default(),
        json!({}),
    )
    .await
    .unwrap();
    yuantuan_core::agent::runtime::control(
        &rig.db_path,
        &id,
        "p_2001",
        "555666",
        "group",
        "amend",
        "增加来源列",
    )
    .await
    .unwrap();
    c.execute("INSERT INTO task_invocations(id,task_id,revision,tool_name,args,state,started_at) VALUES('i_protocol',?1,2,'moderator.probe','{}','started',100)",yuantuan_core::db::params![id]).await.unwrap();
    c.execute(r#"INSERT INTO tool_invocation_records(invocation_id,task_id,revision,call_id,protocol,definition_json,status,error_json,elapsed_ms) VALUES('i_protocol',?1,2,'original_42','native','{}','unknown','{"kind":"timeout","message":"needs verification"}',120000)"#,yuantuan_core::db::params![id]).await.unwrap();
    let (status, body) = rig.get(&format!("/api/tasks/{id}/runtime")).await;
    assert_eq!(status, 200);
    assert_eq!(body["runtime"]["revision"], 2);
    assert_eq!(body["invocations"][0]["call_id"], "original_42");
    assert_eq!(body["invocations"][0]["state"], "started");
    assert_eq!(body["invocations"][0]["status"], "unknown");
    assert_eq!(body["invocations"][0]["error"]["kind"], "timeout");
    assert_eq!(body["brief"]["revision"], 2);
    assert_eq!(body["brief"]["request"]["msg_id"], anchor);
    assert_eq!(body["brief"]["changes"][0]["text"], "增加来源列");
    assert_eq!(body["brief_versions"][0]["event_kind"], "amend");
    c.execute_batch("INSERT INTO capability_invocations(name,kind,chat_id,chat_type,actor,anchor_id,state,started_at,finished_at,summary)
        VALUES('moderator.review','skill','555666','group','p_2001',NULL,'succeeded',90,91,'private-fixture-content'),
        ('moderator-extra.review','skill','555666','group','p_2001',NULL,'succeeded',94,95,NULL);").await.unwrap();
    let (status, history) = rig.get("/api/plugins/moderator/history?from_ts=90&until_ts=101").await;
    assert_eq!(status, 200);
    let items = history["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["status"], "unknown");
    assert_eq!(items[0]["anchor_id"], anchor);
    assert_eq!(items[0]["elapsed_ms"], 120000);
    assert_eq!(items[1]["kind"], "skill");
    assert_eq!(items[1]["elapsed_ms"], 1000);
    assert!(!history.to_string().contains("private-fixture-content"));
    let (_, earlier) = rig.get("/api/plugins/moderator/history?from_ts=90&until_ts=100").await;
    assert_eq!(earlier["items"].as_array().unwrap().len(), 1);
    let (status, _) = rig.get("/api/plugins/moderator/history?from_ts=100&until_ts=90").await;
    assert_eq!(status, 400);
    let private_history = rig.http.get(format!("{}/api/plugins/moderator/history", rig.base)).send().await.unwrap();
    assert_eq!(private_history.status().as_u16(), 401);
    let response = rig
        .http
        .get(format!("{}/api/tasks/{id}/runtime", rig.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);
    let (_, legacy) = rig.get("/api/tasks/t1/runtime").await;
    assert!(legacy["runtime"].is_null());
}

fn temp_dir(prefix: &str) -> PathBuf {
    // nanos + 进程 id + 原子序号：Windows 上 SystemTime 精度只到 ~100ns，同进程两测试并发起 start 可能撞值
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("yt-{prefix}-{}-{seq}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn personality_examples_save_read_rollback_and_apply_to_bot_context() {
    let rig = start().await;
    let (status, defaults) = rig.get("/api/personality/defaults").await;
    assert_eq!(status, 200);
    assert!(defaults["content"]
        .as_str()
        .unwrap()
        .contains("你的搭档是 fei"));
    assert_eq!(defaults["reply_examples"].as_array().unwrap().len(), 12);
    let (_, legacy) = rig.get("/api/personality/versions/1").await;
    assert_eq!(legacy["reply_examples"], json!([]));
    let examples = json!([{"question":"列表怎么排序？", "answer":"使用 sorted() 得到新列表。"}]);
    let response = rig
        .http
        .post(format!("{}/api/personality/versions", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"content":"专业猫型助手", "reply_examples":examples, "note":"示例版"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["version_no"], 2);
    let (_, full) = rig.get("/api/personality/versions/2").await;
    assert_eq!(full["reply_examples"], examples);
    let (_, list) = rig.get("/api/personality/versions").await;
    assert_eq!(list["versions"][0]["reply_examples_count"], 1);

    // 使用实际 API 写入的数据生成回复上下文，证明配置会在下一次回复生效。
    let anchor = yuantuan_core::event::MessageReceivedPayload {
        msg_id: 1,
        chat_id: "555666".into(),
        chat_type: "group".into(),
        sender_pid: "p_2001".into(),
        text: "早".into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 1,
    };
    let context = yuantuan_core::context_builder::build_bot_context(
        &rig.db_path,
        yuantuan_core::state::MoodValue::Calm,
        &anchor,
        &yuantuan_core::context_builder::ContextCfg::default(),
    )
    .await
    .unwrap();
    assert!(context.system.contains("专业猫型助手"));
    assert!(context.system.contains("使用 sorted() 得到新列表。"));
    assert!(!context.user.contains("使用 sorted() 得到新列表。"));

    for (source, version, expected) in [(1, 3, json!([])), (2, 4, examples.clone())] {
        let response = rig
            .http
            .post(format!("{}/api/personality/rollback/{source}", rig.base))
            .bearer_auth(&rig.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let (_, rolled) = rig
            .get(&format!("/api/personality/versions/{version}"))
            .await;
        assert_eq!(rolled["reply_examples"], expected);
        assert_eq!(rolled["active"], true);
    }
    // 旧客户端只传 content 的请求仍然有效。
    let response = rig
        .http
        .post(format!("{}/api/personality/versions", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"content":"旧客户端"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let (_, old_client) = rig.get("/api/personality/versions/5").await;
    assert_eq!(old_client["reply_examples"], json!([]));
    let (_, preserved) = rig.get("/api/personality/versions/2").await;
    assert_eq!(preserved["reply_examples"], examples);
    assert_eq!(preserved["active"], false);
}

#[tokio::test]
async fn personality_rejects_incomplete_or_oversized_examples_without_activating() {
    let rig = start().await;
    for body in [
        json!({"content":" ", "reply_examples":[]}),
        json!({"content":"性格", "reply_examples":[{"question":"问题", "answer":" \n"}]}),
        json!({"content":"性格", "reply_examples":[{"question":" ", "answer":"回答"}]}),
        json!({"content":"性格", "reply_examples":vec![json!({"question":"问题","answer":"回答"});21]}),
        json!({"content":"性格", "reply_examples":[{"question":"长".repeat(1001), "answer":"回答"}]}),
        json!({"content":"性格", "reply_examples":[{"question":"问题", "answer":"长".repeat(4001)}]}),
        json!({"content":"性格", "reply_examples":vec![json!({"question":"问题","answer":"长".repeat(4000)});3]}),
    ] {
        let response = rig
            .http
            .post(format!("{}/api/personality/versions", rig.base))
            .bearer_auth(&rig.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 400);
    }
    let malformed = rig
        .http
        .post(format!("{}/api/personality/versions", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"content":"性格","reply_examples":[{"question":"缺回答"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status().as_u16(), 422);
    let (_, versions) = rig.get("/api/personality/versions").await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 1);
    assert_eq!(versions["versions"][0]["active"], true);
}

async fn seed(db_path: &PathBuf) {
    let mut conn = db::connect(db_path).await.unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // persons（含 self）+ edges + 消息 + 事件 + 长期记忆 + 摘要 + 任务
    conn.execute(
        "INSERT INTO persons(person_id, display_name, first_seen, last_seen) VALUES ('self','云团',?1,?1),('p_2001','小明',?1,?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute_batch(
        "INSERT INTO identities(person_id, platform, platform_uid) VALUES
         ('self', 'qq', '10000'), ('self', 'qq', '10001'),
         ('p_2001', 'other', '2001')",
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO relationship_edges(from_pid, to_pid, trust, familiar, updated_at) VALUES ('p_2001','self',0.7,0.3,?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO messages(chat_id, chat_type, sender_pid, nickname, text, mentions, ts) VALUES ('555666','group','p_2001','小明','早', '[]', ?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO events(kind, payload, ts) VALUES ('DecisionMade', '{\"chat_id\":\"555666\",\"action\":\"reply\"}', ?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO long_memories(owner_type, owner_id, content, source, created_at, updated_at) VALUES ('person','p_2001','小明爱钓鱼','explicit',?1,?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO summaries(owner_type, owner_id, period, date, summary, created_at) VALUES ('chat','555666','daily','2026-10-03','今天很热闹',?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO tasks(task_id, goal, state, budget_max_calls, created_by_pid, chat_id, created_at) VALUES ('t1','写周报','running',10,'p_2001','555666',?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO task_events(task_id, seq, kind, payload, ts) VALUES ('t1',1,'note','\"开工\"',?1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
    conn.execute(
        "INSERT INTO personality_versions(version_no, content, note, created_by, created_at, active) VALUES (1,'你是云团','初始','admin',?1,1)",
        yuantuan_core::db::params![now],
    ).await.unwrap();
}

#[tokio::test]
async fn decision_status_reads_real_states_without_replaying_uncertain_actions() {
    let rig = start().await;
    let mut c = db::connect(&rig.db_path).await.unwrap();
    c.execute_batch("INSERT INTO messages(chat_id,chat_type,sender_pid,nickname,text,ts,decision_state,decision_retry_at,decision_attempts,processed_at) VALUES
      ('42','group','p_2001','群名片','等待重试',1,'pending',4102444800,2,NULL),
      ('42','private','p_2001',NULL,'私聊消息',2,'deciding',0,0,NULL),
      ('42','group','p_2001',NULL,'执行结果未知',3,'uncertain',0,0,4),
      ('42','group','p_2001',NULL,'完成',4,'done',0,0,4),
      ('42','group','self',NULL,'云团自己',5,'pending',0,0,NULL),
      ('42','group','p_2001',NULL,'停止重试的旧消息',7,'failed',0,3,8);
      UPDATE messages SET decision_error='HTTP 400：模型路由不可用' WHERE text='停止重试的旧消息';
      INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,sender_bot) VALUES('42','group','p_2001','其他机器人',6,1);").await.unwrap();
    let (_, data) = rig.get("/api/decisions/status?chat_id=42").await;
    assert_eq!(data["policy"]["group_interval_secs"], 30);
    assert_eq!(data["policy"]["periodic_concurrency"], 2);
    assert_eq!(
        data["counts"],
        json!({"pending":1,"deciding":1,"executing":0,"uncertain":1,"failed":1,"retry_waiting":1})
    );
    assert_eq!(data["messages"].as_array().unwrap().len(), 4);
    assert_eq!(data["messages"][0]["sender_name"], "群名片");
    assert_eq!(data["messages"][1]["chat_type"], "private");
    let (_, uncertain) = rig
        .get("/api/decisions/status?chat_id=42&state=uncertain")
        .await;
    assert_eq!(uncertain["messages"].as_array().unwrap().len(), 1);
    assert_eq!(uncertain["messages"][0]["state"], "uncertain");
    assert_eq!(uncertain["counts"], data["counts"]);
    let (_, failed) = rig
        .get("/api/decisions/status?chat_id=42&state=failed")
        .await;
    assert_eq!(failed["messages"].as_array().unwrap().len(), 1);
    assert_eq!(failed["messages"][0]["error"], "HTTP 400：模型路由不可用");
    assert_eq!(data["policy"]["max_failures"], 3);
    let (_, empty) = rig.get("/api/decisions/status?chat_id=42&limit=0").await;
    assert!(empty["messages"].as_array().unwrap().is_empty());
    assert_eq!(rig.get("/api/decisions/status?state=bad").await.0, 400);
    let unauthorized = rig
        .http
        .get(format!("{}/api/decisions/status", rig.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status().as_u16(), 401);
    let state: String = c
        .query_row(
            "SELECT decision_state FROM messages WHERE text='执行结果未知'",
            yuantuan_core::db::params![],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(state, "uncertain");
}

struct Rig {
    tools: yuantuan_core::tools::Registry,
    http: reqwest::Client,
    base: String,
    token: String,
    db_path: PathBuf,
    self_qq: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

async fn start() -> Rig {
    let dir = temp_dir("pages-api");
    let db_path = dir.join("yuantuan.db");
    {
        let mut conn = db::connect(&db_path).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
    }
    seed(&db_path).await;
    let cfg_path = dir.join("config.toml");
    std::fs::write(
        &cfg_path,
        "[prefilter]\nwindow_secs = 60\nself_msg_cap = 12\n\n[napcat]\ntoken = \"top-secret-token\"\n",
    )
    .unwrap();
    let prov_path = dir.join("providers.toml");
    std::fs::write(
        &prov_path,
        "[provider.mock]\nbase_url = \"http://127.0.0.1:9/v1\"\napi_key_env = \"sk-test-provider-key\"\nmodels = [\"m\"]\n\n[roles]\ndecision = { provider = \"mock\", model = \"m\" }\n",
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut extras = Extras::for_test(cfg_path, prov_path);
    extras.plugin_manager = Some(
        yuantuan_core::plugins::PluginManager::load(
            vec![
                yuantuan_plugin_moderator::plugin(),
                yuantuan_plugin_quark_drive::plugin(),
            ],
            dir.clone(),
            dir.join("plugin-sources"),
            extras.tools_registry.clone(),
            extras.skill_registry.clone(),
            extras.llm_slot.clone(),
        )
        .await
        .unwrap(),
    );
    let tools = extras.tools_registry.clone();
    let self_qq = extras.self_qq.clone();
    let db2 = db_path.clone();
    tokio::spawn(async move {
        let _ = serve(db2, "127.0.0.1", port, extras).await;
    });
    let http = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    // 等服务就绪
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(r) = http.get(format!("{base}/healthz")).send().await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "WebUI 未就绪");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let login: Value = http
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"password": "pw"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    Rig {
        tools,
        http,
        base,
        token: login["token"].as_str().unwrap().to_string(),
        db_path,
        self_qq,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relations_folds_bot_aliases_without_merging_same_name_members_or_history() {
    let rig = start().await;
    let mut details_db = db::connect(&rig.db_path).await.unwrap();
    details_db.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES(100,'555666','group','p_2001','我长期使用 Rust 开发',10,1);").await.unwrap();
    yuantuan_core::memory::update_profile(
        &mut details_db,
        "p_2001",
        "555666",
        100,
        100,
        &yuantuan_core::memory::ProfileUpdate {
            field: yuantuan_core::memory::ProfileField::TechnicalPreferences,
            value: "使用 Rust 开发".into(),
            evidence_msg_id: 100,
            evidence_quote: "长期使用 Rust 开发".into(),
        },
    )
    .await
    .unwrap();
    details_db.execute("INSERT INTO message_images(msg_id,image_index,url,status,observation) VALUES(100,0,'https://example.test/private-url','ready',?1)",yuantuan_core::db::params![json!({"summary":"项目代码截图","visible_text":"Rust","uncertainty":"归属未确认"}).to_string()]).await.unwrap();
    let (status, detail) = rig.get("/api/relations/person/p_2001").await;
    assert_eq!(status, 200);
    assert_eq!(detail["profile"][0]["source_msg_id"], 100);
    assert_eq!(
        detail["visual_observations"][0]["identity_confirmed"],
        false
    );
    assert!(!detail.to_string().contains("private-url"));
    assert_eq!(rig.get("/api/relations/person/missing").await.0, 404);
    let unauthorized = rig
        .http
        .get(format!("{}/api/relations/person/p_2001", rig.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status().as_u16(), 401);
    rig.self_qq
        .store(90001, std::sync::atomic::Ordering::Relaxed);
    let mut conn = db::connect(&rig.db_path).await.unwrap();
    conn.execute_batch(
        "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES
         ('p_90001','云团',0,99),('p_10000','旧机器人',1,90),('p_90002','云团',1,2);
         INSERT INTO identities(person_id,platform,platform_uid) VALUES
         ('p_90001','qq','90001'),('p_90002','qq','90002');
         INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts)
         VALUES ('555666','group','p_90001','旧消息',1);
         UPDATE relationship_edges SET updated_at=1;
         INSERT INTO relationship_edges(from_pid,to_pid,trust,familiar,updated_at) VALUES
         ('self','p_2001',0.1,0.1,1),
         ('p_90001','p_2001',0.8,0.6,2),
         ('p_2001','p_10000',0.9,0.7,2),
         ('p_90001','self',0.5,0.5,3),
         ('p_90002','self',0.4,0.3,4);",
    )
    .await
    .unwrap();
    let (status, graph) = rig.get("/api/relations").await;
    assert_eq!(status, 200);
    let nodes = graph["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert!(nodes
        .iter()
        .all(|n| n["id"] != "p_90001" && n["id"] != "p_10000"));
    assert!(nodes
        .iter()
        .any(|n| n["id"] == "p_90002" && n["name"] == "云团"));
    let me = nodes.iter().find(|n| n["id"] == "self").unwrap();
    assert_eq!(me["qq"], "90001");
    assert_eq!(me["first_seen"], 0);
    assert!(me["last_seen"].as_i64().unwrap() >= 99);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);
    assert!(edges.iter().all(|e| e["from"] != e["to"]));
    let outward = edges
        .iter()
        .find(|e| e["from"] == "self" && e["to"] == "p_2001")
        .unwrap();
    assert_eq!(outward["trust"], 0.8);
    let inward = edges
        .iter()
        .find(|e| e["from"] == "p_2001" && e["to"] == "self")
        .unwrap();
    assert_eq!(inward["trust"], 0.9);

    // Once the handshake persisted ownership, aliases remain folded offline.
    conn.execute_batch(
        "UPDATE identities SET person_id='self' WHERE platform='qq' AND platform_uid='90001'",
    )
    .await
    .unwrap();
    rig.self_qq.store(0, std::sync::atomic::Ordering::Relaxed);
    let (_, offline) = rig.get("/api/relations").await;
    assert_eq!(offline["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(offline["nodes"][0]["qq"], "90001");
    let retained: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE sender_pid='p_90001'",
            yuantuan_core::db::params![],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(
        retained, 1,
        "graph projection must preserve historical messages"
    );
}

impl Rig {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let response = self
            .http
            .post(format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.json().await.unwrap())
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn capability_patch_updates_scope_and_switch_independently() {
    let rig = start().await;
    let name = "fixture.shared";
    let url = format!("{}/api/capabilities", rig.base);
    for kind in ["skill", "tool"] {
        yuantuan_core::capabilities::register(&rig.db_path, name, kind, "fixture", &json!({}))
            .await
            .unwrap();
    }
    let patch = |change: Value| {
        rig.http
            .patch(&url)
            .bearer_auth(&rig.token)
            .json(&change)
            .send()
    };
    let tool = |change: Value| {
        let mut body = json!({"name":name,"kind":"tool"});
        body.as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        body
    };
    assert_eq!(
        rig.http
            .patch(&url)
            .json(&tool(json!({"enabled":false})))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    let (scoped, toggled) = tokio::join!(
        patch(tool(json!({"scopes":["group:42","private:dm_123"]}))),
        patch(tool(json!({"enabled":false}))),
    );
    assert_eq!(scoped.unwrap().status().as_u16(), 200);
    assert_eq!(toggled.unwrap().status().as_u16(), 200);
    for change in [json!({}), json!({"scopes":["group:"]})] {
        assert_eq!(patch(tool(change)).await.unwrap().status().as_u16(), 400);
    }
    let (_, catalog) = rig.get("/api/capabilities").await;
    let find = |kind: &str| {
        catalog["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name && row["kind"] == kind)
            .unwrap()
    };
    assert_eq!(find("tool")["enabled"], false);
    assert_eq!(find("tool")["scopes"], json!(["group:42", "private:dm_123"]));
    assert_eq!(find("skill")["enabled"], true);
    assert_eq!(find("skill")["scopes"], json!([]));
    let cleared: Value = patch(tool(json!({"scopes":[]})))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared, json!({"ok":true,"enabled":false,"scopes":[]}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pages_api_full_pass() {
    let rig = start().await;

    // 无 token 401
    let r = rig
        .http
        .get(format!("{}/api/dashboard", rig.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // Cross-day usage must contribute to all-time totals without inflating today.
    let mut conn = db::connect(&rig.db_path).await.unwrap();
    conn.execute_batch(
        "INSERT INTO llm_usage(ts,role,model,prompt_tokens,completion_tokens,total_tokens) VALUES
        (1,'decision','mock',100,20,120),
        (CAST(strftime('%s','now') AS INTEGER),'bot_chat','mock',10,5,15);",
    )
    .await
    .unwrap();
    drop(conn);
    // 仪表盘
    let (s, d) = rig.get("/api/dashboard").await;
    assert_eq!(s, 200);
    assert_eq!(d["messages_in_today"], 1);
    assert_eq!(d["decision_calls_today"], 1);
    assert_eq!(d["active_tasks"], 1);
    assert_eq!(d["adapter_connected"], false);
    assert_eq!(d["mood"], "calm");
    assert_eq!(d["tokens_today"], 15);
    assert_eq!(d["tokens_total"], 135);
    assert_eq!(d["tokens_in_total"], 110);
    assert_eq!(d["tokens_out_total"], 25);
    assert_eq!(d["llm_calls_total"], 2);
    assert!(d["uptime_secs"].is_u64());

    // events 过滤（kind 命中 + chat_id 命中 + 倒序分页）
    let (s, e) = rig
        .get("/api/events?kind=DecisionMade&chat_id=555666&limit=10")
        .await;
    assert_eq!(s, 200);
    let arr = e["events"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["kind"], "DecisionMade");
    assert_eq!(arr[0]["payload"]["chat_id"], "555666");
    let (s, e2) = rig.get("/api/events?kind=NothingExists").await;
    assert_eq!(s, 200);
    assert_eq!(e2["events"].as_array().unwrap().len(), 0);
    let (_, e3) = rig.get("/api/events?before_id=1").await;
    assert_eq!(e3["events"].as_array().unwrap().len(), 0);

    // 记忆与摘要
    let (_, m) = rig
        .get("/api/memories?owner_type=person&owner_id=p_2001")
        .await;
    let ms = m["memories"].as_array().unwrap();
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0]["content"], "小明爱钓鱼");
    let (_, su) = rig.get("/api/summaries?owner_type=chat").await;
    assert_eq!(
        su["summaries"].as_array().unwrap()[0]["summary"],
        "今天很热闹"
    );

    // 关系网：没有 QQ 身份时不从内部 person_id 猜测帐号。
    let (_, rj) = rig.get("/api/relations").await;
    let nodes = rj["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["id"] == "self"));
    assert!(nodes.iter().any(|n| n["id"] == "p_2001"));
    let peer = nodes.iter().find(|n| n["id"] == "p_2001").unwrap();
    assert_eq!(peer["name"], "小明");
    assert!(peer["qq"].is_null());
    assert!(peer["avatar_url"].is_null());
    assert_eq!(nodes.len(), 2, "多个身份不应重复生成人物节点");
    let me = nodes.iter().find(|n| n["id"] == "self").unwrap();
    assert_eq!(me["qq"], "10001");
    assert_eq!(
        me["avatar_url"],
        "https://q1.qlogo.cn/g?b=qq&nk=10001&s=100"
    );
    let edges = rj["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "p_2001");
    assert!((edges[0]["trust"].as_f64().unwrap() - 0.7).abs() < 1e-9);

    // 配置 GET：napcat.token 掩码、prefilter 原文可读；api_key_present 布尔
    let (_, c) = rig.get("/api/config").await;
    assert_eq!(c["config"]["napcat"]["token"], "***");
    assert_eq!(c["config"]["prefilter"]["window_secs"], 60);
    assert_eq!(
        c["providers"]["provider"]["mock"]["base_url"],
        "http://127.0.0.1:9/v1"
    );
    assert_eq!(c["providers"]["provider"]["mock"]["api_key_env"], "***");
    assert_eq!(c["providers"]["provider"]["mock"]["api_key_present"], true);

    // 任务
    let (_, t) = rig.get("/api/tasks?state=running").await;
    assert_eq!(t["tasks"].as_array().unwrap()[0]["task_id"], "t1");
    let (_, te) = rig.get("/api/tasks/t1/events").await;
    assert_eq!(te["events"].as_array().unwrap()[0]["kind"], "note");
    let r = rig
        .http
        .get(format!("{}/api/tasks/nope/events", rig.base))
        .bearer_auth(&rig.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);

    // 人格：版本列表 → 取全文 → 新建 v2 → rollback v1 得 v3（线性）
    let (_, v1) = rig.get("/api/personality/versions").await;
    assert_eq!(v1["versions"].as_array().unwrap().len(), 1);
    let (_, full) = rig.get("/api/personality/versions/1").await;
    assert_eq!(full["content"], "你是云团");
    let r = rig
        .http
        .post(format!("{}/api/personality/versions", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"content": "新版本人设", "note": "v2 测试"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = rig
        .http
        .post(format!("{}/api/personality/rollback/1", rig.base))
        .bearer_auth(&rig.token)
        .send()
        .await
        .unwrap();
    let rb: Value = r.json().await.unwrap();
    assert_eq!(rb["version_no"], 3);
    let (_, v3) = rig.get("/api/personality/versions/3").await;
    assert_eq!(v3["content"], "你是云团");
    assert_eq!(v3["note"], "回滚自 v1");
    assert_eq!(v3["active"], true);
    // v2 不再 active
    let (_, v2) = rig.get("/api/personality/versions/2").await;
    assert_eq!(v2["active"], false);

    // 插件目录来自运行时，配置校验失败不能覆盖旧配置。
    let (_, plugins) = rig.get("/api/plugins/list").await;
    assert_eq!(plugins["plugins"][0]["name"], "moderator");
    assert_eq!(plugins["plugins"][0]["loaded"], false);
    let mut settings = plugins["plugins"][0]["settings"].clone();
    settings["config"]["operator_qqs"] = json!("123456");
    settings["enabled"] = json!(true);
    let response = rig
        .http
        .post(format!("{}/api/plugins/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"name":"moderator", "settings":settings}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    settings["config"]["operator_qqs"] = json!(12);
    let response = rig
        .http
        .post(format!("{}/api/plugins/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"name":"moderator", "settings":settings}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let (_, plugins) = rig.get("/api/plugins/list").await;
    assert_eq!(
        plugins["plugins"][0]["settings"]["config"]["operator_qqs"],
        "123456"
    );
    assert_eq!(plugins["plugins"][0]["pending_reload"], false);
    assert_eq!(plugins["plugins"][0]["loaded"], true);
    let response = rig
        .http
        .post(format!("{}/api/plugins/toggle", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({"name":"not-compiled", "enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // 知识库占位
    let (_, kb) = rig.get("/api/kb/status").await;
    assert_eq!(kb, json!({"enabled": false, "docs": 0}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_pushes_events_and_config_write_applies() {
    let rig = start().await;

    // tungstenite 握手
    let url = format!("{}/ws?token=bad", rig.base.replace("http", "ws"));
    let bad = tokio_tungstenite::connect_async(url).await;
    assert!(bad.is_err(), "坏 token 应拒绝升级");

    let url = format!("{}/ws?token={}", rig.base.replace("http", "ws"), rig.token);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // POST /api/config（providers 写回 + prefilter 阈值热应用）→ 应收到 ConfigReloaded
    let apply = rig
        .http
        .post(format!("{}/api/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({
            "providers": {
                "provider": { "mock2": { "base_url": "http://127.0.0.1:8/v1", "api_key_env": "", "models": ["x"] } },
                "roles": { "decision": { "provider": "mock2", "model": "x" } }
            },
            "config": { "prefilter": { "window_secs": 45, "self_msg_cap": 9 } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply.status().as_u16(), 200);
    let applied: Value = apply.json().await.unwrap();
    assert!(applied["applied"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap_or("").contains("LLM")));
    assert!(applied["applied"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap_or("").contains("prefilter")));

    // ws 收到 ConfigReloaded（kind 字段）
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = false;
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                if v["kind"] == "ConfigReloaded" {
                    got = true;
                    break;
                }
            }
            other => panic!("ws 异常: {other:?}"),
        }
    }
    assert!(got, "ws 应收到 ConfigReloaded");

    // 关闭帧
    let _ = ws
        .send(tokio_tungstenite::tungstenite::Message::Close(None))
        .await;

    // 结构损坏的 providers 写回被拒（base_url 类型非法，解析不回原结构 → 400）
    let bad = rig
        .http
        .post(format!("{}/api/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({ "providers": { "provider": { "bad": { "base_url": 123 } }, "roles": {} } }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 400);

    // prefilter 阈值已热应用（GET 可见原文）
    let (_, c) = rig.get("/api/config").await;
    assert_eq!(c["config"]["prefilter"]["window_secs"], 45);
    assert_eq!(c["config"]["prefilter"]["self_msg_cap"], 9);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_save_preserves_masked_key_and_selected_role_model() {
    let rig = start().await;

    let response = rig
        .http
        .post(format!("{}/api/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&json!({
            "providers": {
                "provider": {
                    "mock": {
                        "base_url": "http://127.0.0.1:9/v1",
                        "api_key_env": "",
                        "models": ["m", "m2"]
                    }
                },
                "roles": {
                    "decision": { "provider": "mock", "model": "m2" }
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    let (_, config) = rig.get("/api/config").await;
    assert_eq!(
        config["providers"]["provider"]["mock"]["api_key_env"],
        "***"
    );
    assert_eq!(
        config["providers"]["provider"]["mock"]["api_key_present"],
        true
    );
    assert_eq!(config["providers"]["provider"]["mock"]["models"][1], "m2");
    assert_eq!(config["providers"]["roles"]["decision"]["model"], "m2");
}

#[tokio::test]
async fn native_tool_protocol_is_saved_only_for_agent() {
    let rig = start().await;
    let provider =
        json!({"mock":{"base_url":"http://127.0.0.1:9/v1","api_key_env":"","models":["m"]}});
    let payload = json!({"providers":{"provider":provider,"roles":{"agent_exec":{"provider":"mock","model":"m","tool_protocol":"native"},"bot_chat":{"provider":"mock","model":"m"}}}});
    let response = rig
        .http
        .post(format!("{}/api/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let (_, config) = rig.get("/api/config").await;
    assert_eq!(
        config["providers"]["roles"]["agent_exec"]["tool_protocol"],
        "native"
    );
    let mut invalid = payload.clone();
    invalid["providers"]["roles"]["bot_chat"]["tool_protocol"] = json!("native");
    let response = rig
        .http
        .post(format!("{}/api/config", rig.base))
        .bearer_auth(&rig.token)
        .json(&invalid)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let (_, config) = rig.get("/api/config").await;
    assert!(config["providers"]["roles"]["bot_chat"]["tool_protocol"].is_null());
}

#[tokio::test]
async fn connector_probe_uses_authenticated_readonly_registry_and_persistent_switches() {
    use yuantuan_core::tools::*;
    struct Probe;
    #[async_trait::async_trait]
    impl Tool for Probe {
        fn name(&self) -> &'static str {
            "tencent-docs.catalog"
        }
        fn description(&self) -> &'static str {
            "fixture read-only connector"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type":"object","properties":{},"additionalProperties":false})
        }
        fn policy(&self) -> ToolPolicy {
            ToolPolicy {
                effect: ToolEffect::ReadOnly,
                ..Default::default()
            }
        }
        async fn call(&self, ctx: &ToolCtx, _: Value) -> anyhow::Result<ToolOutput> {
            assert_eq!(ctx.chat_id, "plugin-check");
            Ok(ToolOutput {
                summary: "实际 fixture 工具目录".into(),
                data: json!({}),
                artifacts: vec![],
            })
        }
    }
    let rig = start().await;
    let response = rig
        .http
        .post(format!("{}/api/plugins/connection/check", rig.base))
        .json(&json!({"name":"tencent-docs"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);
    assert_eq!(
        rig.post("/api/plugins/connection/check", json!({"name":"qzone"}))
            .await
            .0,
        400
    );
    assert_eq!(
        rig.post(
            "/api/plugins/connection/check",
            json!({"name":"tencent-docs"})
        )
        .await
        .0,
        400
    );
    rig.tools.register(Probe);
    let (status, data) = rig
        .post(
            "/api/plugins/connection/check",
            json!({"name":"tencent-docs"}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(data["summary"], "实际 fixture 工具目录");
    let tool = rig.tools.get("tencent-docs.catalog").unwrap();
    yuantuan_core::capabilities::register(
        &rig.db_path,
        tool.name(),
        "tool",
        tool.description(),
        &tool.parameters_schema(),
    )
    .await
    .unwrap();
    let mut c = db::connect(&rig.db_path).await.unwrap();
    c.execute(
        "UPDATE capability_catalog SET enabled=0 WHERE name='tencent-docs.catalog' AND kind='tool'",
        yuantuan_core::db::params![],
    )
    .await
    .unwrap();
    assert_eq!(
        rig.post(
            "/api/plugins/connection/check",
            json!({"name":"tencent-docs"})
        )
        .await
        .0,
        400
    );
}
#[tokio::test]
async fn connector_probe_reports_account_owner_changes_as_requiring_restart() {
    let rig = start().await;
    let body = json!({"config":{"security":{"super_admin_qq":123}}});
    let (status, result) = rig.post("/api/config", body.clone()).await;
    assert_eq!(status, 200);
    assert!(result["requires_restart"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("security.super_admin_qq"))));
    let (_, same) = rig.post("/api/config", body).await;
    assert!(!same["requires_restart"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("security.super_admin_qq"))));
}

#[tokio::test]
async fn connector_auth_private_quark_onboarding_persists_settings_without_leaking_callback() {
    let rig = start().await;
    let path = rig
        .db_path
        .parent()
        .unwrap()
        .join("connectors/fixture/scripts/quark-drive.cjs");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path,r#"
const fs=require('fs'),args=process.argv.slice(2);
const done=(code,msg)=>{console.log(JSON.stringify({type:'result',code,msg}));process.exit(code===0?0:1)};
if(args[0]==='resolve-agent') done(0,'fixture runtime');
if(args[0]==='get-user-info') done(fs.existsSync('authorized')?0:-1408,'private fixture');
if(args.includes('--token')) {fs.writeFileSync('authorized','fixture');done(0,'ok')}
done(-123,'[前往授权]https://pan.quark.cn/oauth?client_id=official&page_code=opaque');
"#).unwrap();
    let unauth = rig
        .http
        .post(format!("{}/api/plugins/auth/quark-drive", rig.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status().as_u16(), 401);
    assert_eq!(
        rig.post("/api/plugins/auth/not-a-plugin", json!({}))
            .await
            .0,
        400
    );
    let (_, catalog) = rig.get("/api/plugins/list").await;
    let mut settings = catalog["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "quark-drive")
        .unwrap()["settings"]
        .clone();
    settings["config"]["cli_script"] = json!(path);
    settings["allowed_chats"] = json!(["group:42"]);
    settings["timeout_ms"] = json!(23456);
    assert_eq!(
        rig.post(
            "/api/plugins/config",
            json!({"name":"quark-drive","settings":settings})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        rig.post("/api/plugins/auth/quark-drive", json!({})).await.0,
        200
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, status) = rig.get("/api/plugins/auth/quark-drive").await;
        if status["phase"] == "waiting" {
            assert!(status["authorization_url"]
                .as_str()
                .unwrap()
                .starts_with("https://pan.quark.cn/"));
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let login: Value = rig
        .http
        .post(format!("{}/api/auth/login", rig.base))
        .json(&json!({"password":"pw"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let other = rig
        .http
        .post(format!(
            "{}/api/plugins/auth/quark-drive/complete",
            rig.base
        ))
        .bearer_auth(login["token"].as_str().unwrap())
        .json(&json!({"value":"private-code"}))
        .send()
        .await
        .unwrap();
    assert_eq!(other.status().as_u16(), 400);
    assert_eq!(
        rig.post(
            "/api/plugins/auth/quark-drive/complete",
            json!({"value":"https://localhost/?state=missing"})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        rig.post(
            "/api/plugins/auth/quark-drive/complete",
            json!({"value":"https://localhost/?code=private-code"})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        rig.post(
            "/api/plugins/auth/quark-drive/complete",
            json!({"value":"private-code"})
        )
        .await
        .0,
        400
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, status) = rig.get("/api/plugins/auth/quark-drive").await;
        assert!(!status.to_string().contains("private-code"));
        if status["phase"] == "connected" {
            assert!(status["authorization_url"].is_null());
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let (_, catalog) = rig.get("/api/plugins/list").await;
    let plugin = catalog["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "quark-drive")
        .unwrap();
    assert_eq!(plugin["enabled"], true);
    assert_eq!(plugin["loaded"], true);
    assert_eq!(plugin["settings"]["allowed_chats"], json!(["group:42"]));
    assert_eq!(plugin["settings"]["timeout_ms"], 23456);
    assert_eq!(plugin["settings"]["config"]["allow_share_creation"], false);
}
