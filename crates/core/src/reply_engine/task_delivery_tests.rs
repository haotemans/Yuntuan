use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn task_outbox_delivers_once_and_cancelled_revision_never_sends() {
    verify_task_outbox("42", "p_a", None).await;
}

#[tokio::test]
async fn official_task_outbox_uses_original_platform_and_opaque_receipt() {
    verify_task_outbox(
        "qq-official_group_123_room",
        "qq-official_user_123_member",
        Some(crate::platform::QQ_OFFICIAL_ID),
    )
    .await;
}

async fn verify_task_outbox(chat: &'static str, actor: &str, platform: Option<&'static str>) {
    let path = std::env::temp_dir().join(format!("yt-task-outbox-{}.db", rand::random::<u64>()));
    let mut conn = crate::db::connect(&path).await.unwrap();
    crate::db::migrate(&mut conn).await.unwrap();
    conn.execute(
        "INSERT INTO persons VALUES(?1,'A',1,1,NULL)",
        params![actor],
    )
    .await
    .unwrap();
    conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,?1,'group',?2,'请处理任务',1)",params![chat,actor]).await.unwrap();
    let id = crate::agent::create_scoped_task(
        &path,
        &EventBus::default(),
        chat,
        "group",
        actor,
        Some(1),
        "任务",
        crate::agent::TaskPlan::default(),
        json!({}),
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE task_runtime SET phase='finished' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO task_outbox(task_id,revision,text,updated_at) VALUES(?1,1,'实际结果',1)",
        params![id],
    )
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let send: SendFn = Arc::new(move |request| {
        captured.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request.platform_reply.as_ref().map(|route| route.platform),
            platform
        );
        Box::pin(async move {
            if let Some(route) = request.platform_reply {
                assert_eq!(route.chat_id, chat);
                assert_eq!(route.anchor_msg_id, 1);
                Ok(json!({"platform_message_id":"reply!opaque"}))
            } else {
                assert_eq!(request.target, 42);
                Ok(json!({"message_id":9}))
            }
        })
    });
    let eng = Arc::new(ReplyEngine::new(
        path.clone(),
        EventBus::default(),
        send,
        SelfMsgIds::default(),
        Arc::new(std::sync::RwLock::new(ReplyCfg::default())),
        MoodState::default(),
    ));
    let (chat_type, target, mention_qq) = route_conversation("group", chat, actor).unwrap();
    let job = ReplyJob {
        freshness: None,
        chat_id: chat.into(),
        chat_type,
        target,
        anchor_msg_id: 1,
        mention: false,
        mention_qq,
        kind: JobKind::TaskResult {
            task_id: id.clone(),
            revision: 1,
            text: "实际结果".into(),
        },
    };
    process_job(&eng, job.clone()).await;
    process_job(&eng, job.clone()).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let sent: String =
        sqlx::query_scalar("SELECT delivery_state FROM task_runtime WHERE task_id=?")
            .bind(&id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(sent, "sent");
    let receipt: String = sqlx::query_scalar("SELECT receipt FROM task_outbox WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(
        receipt,
        if platform.is_some() {
            "reply!opaque"
        } else {
            "9"
        }
    );
    let bot: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE sender_pid='self' AND reply_anchor_id=1",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(bot, 1);
    conn.execute(
        "UPDATE task_runtime SET revision=2,phase='cancelled' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE task_outbox SET state='ready' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    process_job(&eng, job).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn schema_validator_rejects_invalid_arguments_before_external_call() {
    assert!(crate::capabilities::validate(&json!({"type":"object","required":["count"],"properties":{"count":{"type":"integer","minimum":1,"maximum":4}}}),&json!({"count":99})).is_err());
}
