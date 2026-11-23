use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use yuantuan_core::{
    consolidation::{run_once, ConsolidationCfg, ConsolidationDeps},
    db::{self, params, SqliteExt},
    event::EventBus,
    llm::LlmGateway,
    memory::{self, processing},
    platform::OfficialQqConfig,
};

fn temp() -> PathBuf {
    let p = std::env::temp_dir().join(format!("yt-memory-store-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&p).unwrap();
    p.join("db.sqlite")
}

#[tokio::test]
async fn linked_qq_identities_share_evidenced_group_memory_in_both_directions() {
    let mut conn = db::connect(&temp()).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    conn.execute_batch(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES
            ('p_42',1,1),('qq-official_user_123_member',1,1),('p_77',1,1);
         INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
            (1,'onebot-room','group','p_42','我用 Rust',1),
            (2,'official-room','group','qq-official_user_123_member','我用 Python',2),
            (3,'official-private','private','qq-official_user_123_member','我的私聊秘密',3),
            (4,'onebot-room','group','p_77','我用 Java',4);
         INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at) VALUES
            ('p_42','interests','Rust',1,'我用 Rust',1),
            ('qq-official_user_123_member','work','Python',2,'我用 Python',2),
            ('qq-official_user_123_member','location','秘密',3,'我的私聊秘密',3);
         INSERT INTO memory_items(owner_type,owner_id,item_key,kind,content,status,source_chat_id,source_chat_type,source_end_id,evidence,recorded_at) VALUES
            ('person','p_42','rust','fact','Rust','current','onebot-room','group',1,json_array(json_object('msg_id',1,'quote','我用 Rust')),1),
            ('person','qq-official_user_123_member','python','fact','Python','current','official-room','group',2,json_array(json_object('msg_id',2,'quote','我用 Python')),2),
            ('person','qq-official_user_123_member','secret','fact','秘密','current','official-private','private',3,json_array(json_object('msg_id',3,'quote','我的私聊秘密')),3),
            ('person','qq-official_user_123_member','event','event','群事件','current','official-room','group',2,json_array(json_object('msg_id',2,'quote','我用 Python')),2),
            ('person','p_77','java','fact','Java','current','onebot-room','group',4,json_array(json_object('msg_id',4,'quote','我用 Java')),4);",
    ).await.unwrap();
    let person = "qq-official_user_123_member";
    assert!(memory::semantic::current(&mut conn,"person",person,"official-room","group",4,4).await.unwrap().iter().all(|m| m["content"] != "Rust"));
    let cfg = OfficialQqConfig {
        app_id: "123".into(),
        memory_links: [("member".into(), "42".into())].into(),
        ..Default::default()
    };
    cfg.validate().unwrap();
    cfg.sync_memory_links(&mut conn).await.unwrap();
    let other_scope = processing::Scope::conversation("official-room", "group");
    let batch = processing::claim(&mut conn, other_scope.clone(), 0, 4).await.unwrap().unwrap();
    assert!(processing::fact(&mut conn,&batch,"person",person,"使用 Python",2,"我用 Python").await.unwrap());
    let onebot_scope = processing::Scope::conversation("onebot-room", "group");
    let recalled = memory::recall_scoped(&mut conn,"p_42",&onebot_scope,4,"Python",5).await.unwrap();
    assert_eq!(recalled[0].source_chat_id.as_deref(), Some("official-room"));
    for (owner, chat, expected) in [(person, "official-room", "Rust"), ("p_42", "onebot-room", "Python")] {
        let items = memory::semantic::current(&mut conn,"person",owner,chat,"group",4,4).await.unwrap();
        assert!(items.iter().any(|item| item["content"] == expected));
        assert!(items.iter().all(|item| item["content"] != "秘密" && item["content"] != "Java"));
        if owner == "p_42" {
            assert!(items.iter().all(|item| item["content"] != "群事件"));
        }
        let profile = memory::semantic::scoped_profile(&mut conn,owner,chat,"group",4).await.unwrap();
        assert!(profile.iter().any(|fact| fact.content == expected));
        assert!(profile.iter().all(|fact| fact.content != "秘密"));
    }
    OfficialQqConfig::default().sync_memory_links(&mut conn).await.unwrap();
    assert!(memory::semantic::scoped_profile(&mut conn,"p_42","onebot-room","group",4).await.unwrap().iter().all(|fact| fact.content != "Python"));
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT sender_pid FROM messages WHERE msg_id=2").fetch_one(&mut *conn).await.unwrap(), person);
}

#[tokio::test]
async fn batches_only_advance_after_atomic_commit_and_reject_stale_results() {
    let path = temp();
    let mut conn = db::connect(&path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,2),('b',1,2);
      INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'same','group','a','我长期使用 Rust',1),(2,'same','private','b','我使用 Python',2);").await.unwrap();
    let scope = processing::Scope::conversation("same", "group");
    let batch = processing::claim(&mut conn, scope.clone(), 0, 2)
        .await
        .unwrap()
        .unwrap();
    assert!(processing::claim(&mut conn, scope.clone(), 0, 2)
        .await
        .unwrap()
        .is_none());
    let mut tx = conn.transaction().await.unwrap();
    assert!(!processing::fact(
        &mut tx,
        &batch,
        "person",
        "b",
        "错误归属",
        1,
        "我长期使用 Rust"
    )
    .await
    .unwrap());
    assert!(!processing::fact(
        &mut tx,
        &batch,
        "chat",
        "same",
        "私聊不能混进群",
        2,
        "我使用 Python"
    )
    .await
    .unwrap());
    assert!(processing::fact(
        &mut tx,
        &batch,
        "person",
        "a",
        "长期使用 Rust",
        1,
        "我长期使用 Rust"
    )
    .await
    .unwrap());
    processing::complete(&mut tx, &batch).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM memory_facts")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        0
    );
    conn.execute(
        "UPDATE memory_batches SET lease_until=0 WHERE id=?1",
        params![batch.id],
    )
    .await
    .unwrap();
    let retry = processing::claim(&mut conn, scope.clone(), 0, 2)
        .await
        .unwrap()
        .unwrap();
    let mut tx = conn.transaction().await.unwrap();
    assert!(processing::fact(
        &mut tx,
        &batch,
        "person",
        "a",
        "迟到结果",
        1,
        "我长期使用 Rust"
    )
    .await
    .unwrap());
    assert!(processing::complete(&mut tx, &batch).await.is_err());
    tx.rollback().await.unwrap();
    let mut tx = conn.transaction().await.unwrap();
    assert!(processing::fact(
        &mut tx,
        &retry,
        "person",
        "a",
        "长期使用 Rust",
        1,
        "我长期使用 Rust"
    )
    .await
    .unwrap());
    processing::complete(&mut tx, &retry).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 2);
    assert!(processing::claim(&mut conn, scope, 0, 2)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_memories")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        1
    );
    let recalled = memory::recall(&mut conn, "a", "same", 2, "Rust", 3)
        .await
        .unwrap();
    assert_eq!(
        recalled[0].evidence_quote.as_deref(),
        Some("我长期使用 Rust")
    );
    let private = processing::Scope::conversation("same", "private");
    let batch = processing::claim(&mut conn, private.clone(), 0, 2)
        .await
        .unwrap()
        .unwrap();
    let mut tx = conn.transaction().await.unwrap();
    assert!(processing::fact(
        &mut tx,
        &batch,
        "chat",
        "same",
        "Python 项目记录",
        2,
        "我使用 Python"
    )
    .await
    .unwrap());
    processing::complete(&mut tx, &batch).await.unwrap();
    tx.commit().await.unwrap();
    assert!(memory::recall_scoped(
        &mut conn,
        "a",
        &processing::Scope::conversation("same", "group"),
        2,
        "Python",
        3
    )
    .await
    .unwrap()
    .is_empty());
    assert_eq!(
        memory::recall_scoped(&mut conn, "a", &private, 2, "Python", 3)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn incremental_conversations_resume_failed_batches_without_losing_older_messages() {
    let path = temp();
    let mut conn = db::connect(&path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1);
      INSERT INTO summaries(owner_type,owner_id,period,date,summary,created_at) VALUES('chat','same','daily','2026-11-01','旧摘要保留',1);").await.unwrap();
    for id in 1..=501 {
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,mentions,ts) VALUES(?1,'same','group','a',?2,?3,1000)",params![id,format!("讨论记录 {id}"),if id==1{"[\"b\"]"}else{"[]"}]).await.unwrap();
    }
    conn.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(502,'same','private','a','私聊内容独立',1000)").await.unwrap();
    let fail = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let failed = fail.clone();
    let seen = calls.clone();
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(move|axum::Json(body):axum::Json<Value>|{
        let fail=failed.clone();let calls=seen.clone();async move {
            let input:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap().split("\n\n上一次").next().unwrap()).unwrap();
            calls.lock().unwrap().push(input.clone());
            let content=if fail.load(Ordering::SeqCst){"invalid".to_owned()}else{json!({"chat_summary":format!("{} 的本批讨论",input["chat_type"].as_str().unwrap()),"person_facts":[],"group_facts":[],"rel_events":[]}).to_string()};
            axum::Json(json!({"choices":[{"message":{"content":content}}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = path.with_file_name("providers.toml");
    std::fs::write(&provider,format!("[provider.mock]\nbase_url=\"http://{address}\"\n[roles]\ndecision={{provider=\"mock\",model=\"test\"}}\n")).unwrap();
    let deps = ConsolidationDeps {
        db_path: path.clone(),
        llm: Arc::new(std::sync::RwLock::new(Some(Arc::new(
            LlmGateway::load(&provider).unwrap(),
        )))),
        bus: EventBus::new(16),
        self_qq: Arc::new(AtomicU64::new(0)),
        cfg: ConsolidationCfg::default(),
    };
    run_once(&deps).await;
    let scope = processing::Scope::conversation("same", "group");
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 0);
    fail.store(false, Ordering::SeqCst);
    run_once(&deps).await;
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 501);
    assert_eq!(
        processing::cursor(
            &mut conn,
            &processing::Scope::conversation("same", "private")
        )
        .await
        .unwrap(),
        502
    );
    run_once(&deps).await;
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 501);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM memory_diary_entries")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT summary FROM summaries")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        "旧摘要保留"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        502
    );
    assert_eq!(
        sqlx::query_scalar::<_, f64>(
            "SELECT familiar FROM relationship_edges WHERE from_pid='a' AND to_pid='b'"
        )
        .fetch_one(&mut *conn)
        .await
        .unwrap(),
        0.01
    );
    assert_eq!(calls.lock().unwrap()[0]["messages"][0]["msg_id"], 1);
    server.abort();
}

#[tokio::test]
async fn conversation_daily_budget_survives_failure_and_reconnection() {
    let path = temp();
    let mut conn = db::connect(&path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    let scope = processing::Scope::conversation("a", "group");
    let batch = processing::claim_limited(&mut conn, scope.clone(), 0, 1, Some(1))
        .await
        .unwrap()
        .unwrap();
    processing::fail(&mut conn, &batch).await.unwrap();
    drop(conn);
    let mut conn = db::connect(&path).await.unwrap();
    assert!(
        processing::claim_limited(&mut conn, scope.clone(), 0, 1, Some(1))
            .await
            .unwrap()
            .is_none()
    );
    assert!(processing::claim_limited(
        &mut conn,
        processing::Scope::conversation("b", "private"),
        0,
        2,
        Some(1)
    )
    .await
    .unwrap()
    .is_none());
    assert_eq!(processing::cursor(&mut conn, &scope).await.unwrap(), 0);
    // Conversation attempts do not consume the independent portrait allowance.
    assert!(
        processing::claim_limited(&mut conn, processing::Scope::portrait("a"), 0, 1, Some(1))
            .await
            .unwrap()
            .is_some()
    );
}
