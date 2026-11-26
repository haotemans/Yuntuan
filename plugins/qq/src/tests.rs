use super::*;
use std::sync::Mutex;

struct MockHost {
    owner: Option<u64>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl MockHost {
    fn new(owner: Option<u64>) -> Self {
        Self {
            owner,
            calls: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl HostServices for MockHost {
    fn account_owner_qq(&self) -> Option<u64> {
        self.owner
    }
    fn protocol_account(&self, protocol: &str) -> Option<String> {
        match protocol {
            "onebot" => Some("10001".into()),
            "qq_official" => Some("102030".into()),
            _ => None,
        }
    }
    fn is_protocol_connected(&self, protocol: &str) -> bool {
        matches!(protocol, "onebot" | "qq_official")
    }
    async fn call_protocol(
        &self,
        _ctx: &ToolCtx,
        protocol: &str,
        action: &str,
        params: Value,
    ) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((protocol.to_string(), action.to_string(), params.clone()));
        Ok(match (protocol, action) {
            ("qq_official", "get_status") => json!({
                "platform": "qq_official",
                "app_id": "102030",
                "state": "online",
                "connected": true
            }),
            (_, "get_group_info") => json!({
                "group_id": params["group_id"],
                "group_name": "云团测试群",
                "member_count": 12,
                "max_member_count": 500
            }),
            "get_group_member_info" => json!({
                "user_id": params["user_id"],
                "nickname": "小团",
                "card": "群友小团",
                "role": "member"
            }),
            "get_essence_msg_list" => json!([{"message_id": 1}]),
            _ => json!({"ok": true}),
        })
    }
    async fn chat(&self, _ctx: &ToolCtx, _system: &str, _user: &str) -> Result<String> {
        Ok(String::new())
    }
    async fn authorize_tool_for_skill(&self, _ctx: &ToolCtx, _name: &str, _args: &Value) -> Result<()> {
        Ok(())
    }
    async fn read_data(&self, _key: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn write_data(&self, _key: &str, _value: Value) -> Result<()> {
        Ok(())
    }
    fn log(&self, _message: &str) {}
}

fn group_ctx(sender: &str) -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "888888".into(),
        chat_type: "group".into(),
        sender_pid: sender.into(),
        locale: None,
    }
}

#[tokio::test]
async fn qq_plugin_calls_onebot_protocol_for_group_info_and_poke() {
    let host = Arc::new(MockHost::new(Some(10001)));
    let caps = plugin()
        .start(
            PluginContext { host: host.clone() },
            json!({"allow_poke": true, "allow_group_queries": true, "allow_owner_notice": false}),
        )
        .await
        .unwrap();

    let group_info = caps.tools.iter().find(|t| t.name() == "qq.group_info").unwrap();
    let out = group_info.call(&group_ctx("p_20002"), json!({})).await.unwrap();
    assert!(out.summary.contains("云团测试群"));

    let poke = caps.tools.iter().find(|t| t.name() == "qq.poke").unwrap();
    let out = poke
        .call(&group_ctx("p_20002"), json!({"user_id": 30003}))
        .await
        .unwrap();
    assert!(out.summary.contains("30003"));

    let calls = host.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "onebot");
    assert_eq!(calls[0].1, "get_group_info");
    assert_eq!(calls[0].2["group_id"], 888888);
    assert_eq!(calls[1].1, "send_poke");
    assert_eq!(calls[1].2, json!({"group_id": 888888, "user_id": 30003}));
}

#[tokio::test]
async fn qq_notice_requires_config_flag_and_account_owner() {
    let host = Arc::new(MockHost::new(Some(10001)));
    let caps = plugin()
        .start(
            PluginContext { host: host.clone() },
            json!({"allow_poke": false, "allow_group_queries": true, "allow_owner_notice": true}),
        )
        .await
        .unwrap();

    let notice = caps.tools.iter().find(|t| t.name() == "qq.group_notice").unwrap();
    assert!(notice
        .call(&group_ctx("p_20002"), json!({"content": "hi"}))
        .await
        .is_err());
    assert!(notice
        .call(&group_ctx("p_10001"), json!({"content": "群规更新"}))
        .await
        .is_ok());

    let poke = caps.tools.iter().find(|t| t.name() == "qq.poke").unwrap();
    assert!(poke.call(&group_ctx("p_10001"), json!({})).await.is_err());
}

#[tokio::test]
async fn qq_plugin_rejects_non_onebot_channel() {
    let host = Arc::new(MockHost::new(Some(10001)));
    let caps = plugin()
        .start(
            PluginContext { host },
            json!({"allow_poke": true, "allow_group_queries": true, "allow_owner_notice": false}),
        )
        .await
        .unwrap();

    let official_ctx = ToolCtx {
        task_id: None,
        chat_id: "qq-official_group_abc".into(),
        chat_type: "group".into(),
        sender_pid: "p_10001".into(),
        locale: None,
    };
    let poke = caps.tools.iter().find(|t| t.name() == "qq.poke").unwrap();
    assert!(!poke.available_in_chat(&official_ctx));
    assert!(poke.call(&official_ctx, json!({"user_id": 10001})).await.is_err());

    let qzone_pub = caps
        .tools
        .iter()
        .find(|t| t.name() == "qq.qzone_publish")
        .unwrap();
    assert!(!qzone_pub.available_in_chat(&official_ctx));

    let recall = caps.tools.iter().find(|t| t.name() == "qq.recall").unwrap();
    assert!(!recall.available_in_chat(&official_ctx));

    let status = caps
        .tools
        .iter()
        .find(|t| t.name() == "qq.channel_status")
        .unwrap();
    assert!(status.available_in_chat(&official_ctx));
    let official_status = status.call(&official_ctx, json!({})).await.unwrap();
    assert!(official_status.summary.contains("QQ 官方机器人"));
    assert_eq!(official_status.data["app_id"], "102030");

    let group_info = caps
        .tools
        .iter()
        .find(|t| t.name() == "qq.group_info")
        .unwrap();
    assert!(group_info.available_in_chat(&official_ctx));
    let official_group = group_info.call(&official_ctx, json!({})).await.unwrap();
    assert_eq!(official_group.data["platform"], "qq_official");

    let onebot_status = status.call(&group_ctx("p_10001"), json!({})).await.unwrap();
    assert!(onebot_status.summary.contains("OneBot QQ"));
}
