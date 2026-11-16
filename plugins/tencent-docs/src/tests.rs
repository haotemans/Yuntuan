use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
#[derive(Default)]
struct FakeRemote {
    calls: AtomicUsize,
    fail: AtomicBool,
}
#[async_trait]
impl Remote for FakeRemote {
    async fn catalog(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"name":"get_content","inputSchema":{"type":"object","properties":{"file_id":{"type":"string"}},"required":["file_id"],"additionalProperties":false}}),
            json!({"name":"create_word_by_markdown","inputSchema":{"type":"object","required":["markdown"],"properties":{"markdown":{"type":"string"}}}}),
            json!({"name":"delete_space_node","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object"}}),
        ])
    }
    async fn invoke(&self, _: &str, _: Value) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ensure!(!self.fail.load(Ordering::SeqCst), "fake upstream secret");
        Ok(json!({"content":[{"type":"text","text":"fixture 文档原文"}]}))
    }
}
struct Host {
    enabled: AtomicBool,
    choice: Mutex<Value>,
    checks: Mutex<Vec<String>>,
}
#[async_trait]
impl HostServices for Host {
    fn account_owner_qq(&self) -> Option<u64> {
        Some(123)
    }
    async fn authorize_tool_for_skill(&self, _: &ToolCtx, name: &str, args: &Value) -> Result<()> {
        self.checks.lock().unwrap().push(name.into());
        ensure!(self.enabled.load(Ordering::SeqCst), "工具已停用");
        ensure!(args.is_object(), "invalid object");
        Ok(())
    }
    async fn chat(&self, _: &ToolCtx, _: &str, _: &str) -> Result<String> {
        Ok(self.choice.lock().unwrap().to_string())
    }
    async fn read_data(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn write_data(&self, _: &str, _: Value) -> Result<()> {
        Ok(())
    }
    fn log(&self, _: &str) {}
}
fn ctx(kind: &str) -> ToolCtx {
    ToolCtx {
        task_id: Some("task".into()),
        chat_id: "42".into(),
        chat_type: kind.into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}
#[test]
fn unsafe_config_and_destructive_annotations_are_rejected() {
    assert!(TencentDocs
        .validate_config(&json!({"endpoint":"https://evil.test/mcp"}))
        .is_err());
    assert!(TencentDocs
        .validate_config(&json!({"token_env":"bad-name"}))
        .is_err());
    assert!(TencentDocs
        .validate_config(&json!({"token_env":"1INVALID"}))
        .is_err());
    assert!(TencentDocs
        .validate_config(&json!({"token_env":"TENCENT_DOCS_TOKEN"}))
        .is_ok());
    assert!(!read_only(
        "delete_space_node",
        &json!({"annotations":{"readOnlyHint":true}})
    ));
    assert!(!read_only(
        "get_content",
        &json!({"annotations":{"readOnlyHint":false}})
    ));
}
#[tokio::test]
async fn schemas_read_write_routes_and_uncertain_writes_have_no_implicit_retry() {
    let remote = Arc::new(FakeRemote::default());
    let connector = Connector {
        host: fake_host(),
        remote: remote.clone(),
        allow_writes: true,
    };
    let catalog = connector.catalog(&ctx("private")).await.unwrap();
    assert!(connector
        .invoke(&ctx("private"), "get_content", json!({}), &catalog, true)
        .await
        .is_err());
    assert!(connector
        .invoke(
            &ctx("group"),
            "create_word_by_markdown",
            json!({"markdown":"hello"}),
            &catalog,
            false
        )
        .await
        .is_err());
    assert!(connector
        .invoke(
            &ctx("private"),
            "create_word_by_markdown",
            json!({"markdown":"hello"}),
            &catalog,
            true
        )
        .await
        .is_err());
    assert!(connector
        .invoke(
            &ctx("private"),
            "delete_space_node",
            json!({}),
            &catalog,
            false
        )
        .await
        .is_err());
    assert!(connector
        .invoke(&ctx("private"), "invented", json!({}), &catalog, false)
        .await
        .is_err());
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
    assert!(connector
        .invoke(
            &ctx("group"),
            "get_content",
            json!({"file_id":"known"}),
            &catalog,
            true
        )
        .await
        .is_ok());
    remote.fail.store(true, Ordering::SeqCst);
    let error = connector
        .invoke(
            &ctx("private"),
            "create_word_by_markdown",
            json!({"markdown":"hello"}),
            &catalog,
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ToolError>().unwrap().kind,
        ToolErrorKind::ResultUnknown
    );
    assert!(!error.to_string().contains("secret"));
    assert_eq!(remote.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn skill_checks_catalog_and_chosen_tool_and_renders_real_content() {
    let remote = Arc::new(FakeRemote::default());
    let connector = Arc::new(Connector {
        host: fake_host(),
        remote: remote.clone(),
        allow_writes: false,
    });
    let host = Arc::new(Host {
        enabled: AtomicBool::new(false),
        choice: Mutex::new(json!({"name":"get_content","arguments":{"file_id":"known"}})),
        checks: Mutex::new(vec![]),
    });
    let skill = DocsSkill {
        connector: connector.clone(),
        host: host.clone(),
    };
    assert!(skill
        .invoke(&ctx("private"), json!({"user_text":"读文档"}))
        .await
        .is_err());
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
    host.enabled.store(true, Ordering::SeqCst);
    let out = skill
        .invoke(&ctx("group"), json!({"user_text":"读文档"}))
        .await
        .unwrap();
    assert_eq!(out.summary, "fixture 文档原文");
    assert_eq!(
        host.checks.lock().unwrap().last().unwrap(),
        "tencent-docs.read"
    );
    let catalog = skill
        .invoke(&ctx("private"), json!({"user_text":"工具"}))
        .await
        .unwrap();
    assert!(catalog.summary.contains("get_content"));
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        DocsTool {
            connector,
            op: Op::Read
        }
        .policy()
        .effect,
        ToolEffect::ReadOnly
    );
}

fn fake_host() -> Arc<Host> {
    Arc::new(Host {
        enabled: AtomicBool::new(true),
        choice: Mutex::new(json!({})),
        checks: Mutex::new(vec![]),
    })
}
#[tokio::test]
async fn non_owner_cannot_discover_or_read_the_connected_account() {
    let remote = Arc::new(FakeRemote::default());
    let connector = Connector {
        remote: remote.clone(),
        allow_writes: true,
        host: fake_host(),
    };
    let other = ToolCtx {
        sender_pid: "p_456".into(),
        ..ctx("private")
    };
    assert!(connector.catalog(&other).await.is_err());
    let catalog = connector.catalog(&ctx("private")).await.unwrap();
    assert!(connector
        .invoke(
            &other,
            "get_content",
            json!({"file_id":"known"}),
            &catalog,
            true
        )
        .await
        .is_err());
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
}
