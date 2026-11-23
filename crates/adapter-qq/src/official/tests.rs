use super::*;
use axum::{
    extract::{ws::Message, Path, WebSocketUpgrade},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use yuantuan_core::{event::Event, reply_engine::PlatformReply};

#[tokio::test]
async fn official_gateway_and_passive_replies_keep_namespaces_and_receipts() {
    let path = std::env::temp_dir().join(format!("yt-official-{}.db", std::process::id()));
    let mut c = db::connect(&path).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    let timestamp: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now','-10 minutes')")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let database = c.database();
    drop(c);
    let bus = EventBus::default();
    let mut rx = bus.subscribe();
    let router = Router::new()
        .route("/socket", get(move |upgrade: WebSocketUpgrade| {
            let timestamp = timestamp.clone();
            async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    socket.send(Message::Text(json!({"op":10,"d":{"heartbeat_interval":500}}).to_string().into())).await.unwrap();
                    let request = socket.recv().await.unwrap().unwrap();
                    let identify: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
                    assert_eq!(identify, json!({"op":2,"d":{"token":"QQBot access","intents":33554432,"shard":[0,1]}}));
                    for event in [
                        json!({"op":0,"t":"READY","s":1,"d":{"session_id":"session-one"}}),
                        json!({"op":0,"t":"GROUP_AT_MESSAGE_CREATE","s":2,"d":{"id":"group-message!","timestamp":timestamp,
                            "group_openid":"room","author":{"member_openid":"member"},"content":"<@!99> 你好"}}),
                        json!({"op":0,"t":"GROUP_MESSAGE_CREATE","s":3,"d":{"id":"group-message!","timestamp":timestamp,
                            "group_openid":"room","author":{"member_openid":"member"},"content":"重复投递","mentions":[{"is_you":true}]}}),
                        json!({"op":0,"t":"C2C_MESSAGE_CREATE","s":4,"d":{"id":"private-message!","timestamp":timestamp,
                            "author":{"user_openid":"member"},"content":"私聊"}}),
                        json!({"op":0,"t":"GROUP_MESSAGE_CREATE","s":5,"d":{"id":"regular-message!","timestamp":timestamp,
                            "group_openid":"room","author":{"member_openid":"member","member_nickname":"群名片"},"content":"<@other> 普通聊天",
                            "mentions":[{"member_openid":"other","is_you":false}],"attachments":[{"content_type":"image/png","url":"https://multimedia.nt.qq.com.cn/download?example=1"}]}}),
                        json!({"op":0,"t":"GROUP_MESSAGE_CREATE","s":6,"d":{"id":"mentioned-message!","timestamp":timestamp,
                            "group_openid":"room","author":{"member_openid":"member"},"content":"<@thisBot> 全量里也能 @",
                            "mentions":[{"member_openid":"thisBot","is_you":true}]}}),
                    ] {
                        socket.send(Message::Text(event.to_string().into())).await.unwrap();
                    }
                    let heartbeat = socket.recv().await.unwrap().unwrap();
                    let payload: Value = serde_json::from_str(heartbeat.to_text().unwrap()).unwrap();
                    assert_eq!(payload, json!({"op":1,"d":6}));
                    socket.send(Message::Text(json!({"op":11}).to_string().into())).await.unwrap();
                })
            }
        }))
        .route("/token", post(|Json(body): Json<Value>| async move {
            assert_eq!(body, json!({"appId":"123","clientSecret":"fixture-secret"}));
            Json(json!({"access_token":"access","expires_in":"7200"}))
        }))
        .route("/gateway", get(|| async { Json(json!({"url":"wss://api.bot.qq.com/websocket/"})) }))
        .route("/v2/{kind}/{recipient}/messages", post(
            |Path((kind, recipient)): Path<(String, String)>, headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "QQBot access");
                assert_eq!(body["msg_type"], 0);
                assert_eq!(body["content"], "回复");
                if kind == "groups" {
                    assert_eq!(recipient, "room");
                    assert_eq!(body["msg_id"], "group-message!");
                } else {
                    assert_eq!(kind, "users");
                    assert_eq!(recipient, "member");
                    assert_eq!(body["msg_id"], "private-message!");
                }
                Json(json!({"id":format!("{kind}-reply-{}",body["msg_seq"])}))
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut adapter = OfficialQq::new(
        OfficialQqConfig {
            enabled: true,
            app_id: "123".into(),
            app_secret: "fixture-secret".into(),
            ..OfficialQqConfig::default()
        },
        path.clone(),
        bus,
    )
    .unwrap();
    let inner = Arc::get_mut(&mut adapter.0).unwrap();
    inner.token_url = format!("http://{address}/token");
    inner.api_root = format!("http://{address}");
    assert_eq!(adapter.gateway_url("access").await.unwrap(), "wss://api.bot.qq.com/websocket/");
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}/socket"))
        .await
        .unwrap();
    let gateway = adapter.clone();
    let gateway = tokio::spawn(async move {
        let token = gateway.token().await.unwrap();
        let mut session = gateway::Session::default();
        assert!(gateway
            .run_socket(&mut ws, &mut session, &token)
            .await
            .is_err());
    });
    let Event::MessageReceived(group) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected group message");
    };
    let Event::MessageReceived(private) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected private message");
    };
    assert_eq!(adapter.status().state, "online");
    assert_eq!(group.text, "你好");
    assert!(group.at_me);
    assert_eq!(group.sender_pid, private.sender_pid);
    assert_eq!(group.chat_id, "qq-official_group_123_room");
    assert_eq!(private.chat_id, "qq-official_private_123_member");
    let Event::MessageReceived(regular) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await.unwrap().unwrap()
    else {
        panic!("expected ordinary group message");
    };
    let Event::MessageReceived(mentioned) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await.unwrap().unwrap()
    else {
        panic!("expected mentioned group message");
    };
    assert_eq!(regular.chat_id, group.chat_id);
    assert_eq!(regular.sender_pid, group.sender_pid);
    assert!(!regular.at_me);
    assert_eq!(regular.text, "<@other> 普通聊天");
    assert_eq!(regular.image_urls, ["https://multimedia.nt.qq.com.cn/download?example=1"]);
    assert!(mentioned.at_me);
    assert_eq!(mentioned.text, "全量里也能 @");
    let request = |chat_type, chat_id, anchor_msg_id| SendRequest {
        chat_type,
        target: 0,
        operation: SendOperation::Message,
        platform_reply: Some(PlatformReply {
            platform: QQ_OFFICIAL_ID,
            chat_id,
            anchor_msg_id,
        }),
        segments: json!([{"type":"text","data":{"text":"回复"}}]),
    };
    assert!(adapter
        .send(request(
            ChatType::Private,
            private.chat_id.clone(),
            group.msg_id
        ))
        .await
        .is_err());
    let group_req = request(ChatType::Group, group.chat_id, group.msg_id);
    for seq in 1..=5 {
        assert_eq!(
            adapter.send(group_req.clone()).await.unwrap()["platform_message_id"],
            format!("groups-reply-{seq}")
        );
    }
    assert!(adapter.send(group_req).await.is_err());
    let private_req = request(ChatType::Private, private.chat_id, private.msg_id);
    for seq in 1..=4 {
        assert_eq!(adapter.send(private_req.clone()).await.unwrap()["platform_message_id"], format!("users-reply-{seq}"));
    }
    assert!(adapter.send(private_req).await.is_err());
    tokio::time::timeout(Duration::from_secs(5), gateway)
        .await
        .unwrap()
        .unwrap();
    let mut c = db::connect(&path).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        4
    );
    let (nickname, mentions): (String, String) = sqlx::query_as("SELECT nickname,mentions FROM messages WHERE msg_id=?")
        .bind(regular.msg_id).fetch_one(&mut *c).await.unwrap();
    assert_eq!(nickname, "群名片");
    assert_eq!(mentions, r#"["qq-official_user_123_other"]"#);
    drop(c);
    server.abort();
    database.close().await;
    let _ = std::fs::remove_file(path);
}
