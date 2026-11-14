use serde_json::json;
use yuantuan_core::{
    db::{self, params, SqliteExt},
    event::MessageReceivedPayload,
    working_context::{self, Update},
};

async fn fixture() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("yt-working-{}.db", rand::random::<u64>()));
    let mut c = db::connect(&p).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    c.execute_batch(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1)",
    )
    .await
    .unwrap();
    p
}
async fn message(p: &std::path::Path, text: &str) -> MessageReceivedPayload {
    let mut c = db::connect(p).await.unwrap();
    c.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('42','group','a',?1,unixepoch())", params![text]).await.unwrap();
    MessageReceivedPayload {
        msg_id: c.last_insert_rowid().await.unwrap(),
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "a".into(),
        text: text.into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: yuantuan_core::window::now(),
    }
}
fn update(v: serde_json::Value) -> Update {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn router_accepts_source_backed_memory_and_next_snapshot_receives_it() {
    use yuantuan_core::{
        context_builder::{self, ContextCfg},
        decision,
        event::EventBus,
        llm::LlmGateway,
        state::MoodState,
    };
    let p = fixture().await;
    let msg = message(&p, "云团帮我部署模型，只能本地运行").await;
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(|axum::Json(body):axum::Json<serde_json::Value>|async move{
        let ctx:serde_json::Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let raw=json!({"action":"reply","mood":"calm","mention":false,"reply_len":"short","reason":"记录当前要求","working_memory":{"goal_quote":"部署模型","conditions":[{"key":"环境","quote":"只能本地运行"}]},"assessment":{"audience":"bot","intent":"request","continuity":"new_topic","confidence":"high","benefit":"high","novelty":"new","evidence":"sufficient","evidence_msg_ids":[ctx["anchor"]["msg_id"]]}}).to_string();
        axum::Json(json!({"choices":[{"message":{"content":raw},"finish_reason":"stop"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let providers = p.with_extension("toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url=\"http://{addr}\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gw = LlmGateway::load(&providers).unwrap();
    let snap =
        context_builder::capture_reply_snapshot(&p, &msg, msg.msg_id, &ContextCfg::default())
            .await
            .unwrap();
    let out = decision::decide(
        &p,
        &gw,
        &EventBus::default(),
        &MoodState::default(),
        &msg,
        None,
        &snap,
    )
    .await;
    assert_eq!(
        out.output.focus_context_id,
        Some(msg.msg_id),
        "{:?}",
        out.output
    );
    let next = message(&p, "显卡是4090").await;
    let snap =
        context_builder::capture_reply_snapshot(&p, &next, next.msg_id, &ContextCfg::default())
            .await
            .unwrap();
    assert_eq!(
        snap.working_context[0]["conditions"][0]["quote"],
        "只能本地运行"
    );
    let mut c = db::connect(&p).await.unwrap();
    let facts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM person_profile_facts")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(facts, 0);
    server.abort();
}

#[tokio::test]
async fn conditions_survive_followups_and_corrections_preserve_provenance_and_versions() {
    let p = fixture().await;
    let first = message(&p, "帮我部署模型，使用4090，只能本地运行").await;
    let start = update(
        json!({"goal_quote":"部署模型","conditions":[{"key":"显卡","quote":"4090"},{"key":"环境","quote":"只能本地运行"}]}),
    );
    let id = working_context::apply(&p, &first, &start).await.unwrap();
    assert_eq!(
        working_context::apply(&p, &first, &start).await.unwrap(),
        id
    );
    let next = message(&p, "改用5090，结果整理成表格").await;
    let change = update(
        json!({"context_id":id,"expected_revision":1,"conditions":[{"key":"显卡","quote":"5090"},{"key":"格式","quote":"整理成表格"}]}),
    );
    working_context::apply(&p, &next, &change).await.unwrap();
    let mut c = db::connect(&p).await.unwrap();
    let latest = working_context::view(&mut c, &next, next.msg_id)
        .await
        .unwrap();
    assert_eq!(latest[0]["revision"], 2);
    assert_eq!(latest[0]["goal"]["source_msg_id"], first.msg_id);
    assert_eq!(latest[0]["conditions"].as_array().unwrap().len(), 3);
    assert!(latest[0]["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["quote"] == "只能本地运行"));
    assert!(latest[0]["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["quote"] == "5090" && v["source_msg_id"] == next.msg_id));
    let old = working_context::view(&mut c, &next, first.msg_id)
        .await
        .unwrap();
    assert_eq!(old[0]["revision"], 1);
    assert_eq!(old[0]["conditions"][0]["quote"], "4090");
    assert_eq!(latest[0]["execution_authorization"], false);
    let third = message(&p, "改用3090").await;
    assert!(working_context::apply(&p,&third,&update(json!({"context_id":id,"expected_revision":1,"conditions":[{"key":"显卡","quote":"3090"}]}))).await.is_err());
    db::migrate(&mut c).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM working_context_versions")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn memory_rejects_invented_quotes_wrong_author_and_cross_scope_and_future_reads() {
    let p = fixture().await;
    let first = message(&p, "帮我部署模型").await;
    assert!(
        working_context::apply(&p, &first, &update(json!({"goal_quote":"去购买显卡"})))
            .await
            .is_err()
    );
    let id = working_context::apply(&p, &first, &update(json!({"goal_quote":"部署模型"})))
        .await
        .unwrap();
    let next = message(&p, "改用4090").await;
    for (kind, owner) in [("private", "a"), ("group", "b")] {
        let mut wrong = next.clone();
        wrong.chat_type = kind.into();
        wrong.sender_pid = owner.into();
        assert!(working_context::apply(&p,&wrong,&update(json!({"context_id":id,"expected_revision":1,"conditions":[{"key":"显卡","quote":"4090"}]}))).await.is_err());
        let mut c = db::connect(&p).await.unwrap();
        assert!(working_context::view(&mut c, &wrong, next.msg_id)
            .await
            .unwrap()
            .is_empty());
    }
    let mut c = db::connect(&p).await.unwrap();
    assert!(working_context::view(&mut c, &next, first.msg_id - 1)
        .await
        .unwrap()
        .is_empty());
    let mut old = first.clone();
    old.ts -= 31 * 86400;
    assert!(working_context::view(&mut c, &old, next.msg_id)
        .await
        .unwrap()
        .is_empty());
    let mut altered = next.clone();
    altered.text = "其他原话".into();
    assert!(
        working_context::apply(&p, &altered, &update(json!({"goal_quote":"其他原话"})))
            .await
            .is_err()
    );
}
