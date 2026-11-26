//! Real plugin/host/registry composition with a fake external connector, never QQ publication.
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, RwLock};
use yuantuan_core::{
    db::{self, params, SqliteExt},
    plugins::{PluginManager, PluginSettings},
    skills::SkillRegistry,
    tools::Registry,
};
use yuantuan_plugin_api::*;

#[derive(Default)]
struct Connector {
    published: Mutex<Vec<QzonePost>>,
}
#[async_trait]
impl QzoneService for Connector {
    async fn execute(&self, ctx: &ToolCtx, action: QzoneAction) -> Result<Value> {
        ensure!(
            ctx.sender_pid == "p_123",
            ToolError::new(ToolErrorKind::PermissionDenied, "非超级管理员")
        );
        match action {
            QzoneAction::Status | QzoneAction::CheckAccess { .. } => {
                Ok(json!({"qq":999,"connected":true}))
            }
            QzoneAction::Publish { post, account_qq } => {
                ensure!(account_qq == 999, "账号不匹配");
                self.published.lock().unwrap().push(post);
                Ok(json!({"status":"published","tid":"verified-id","qq":999}))
            }
        }
    }
}
fn context() -> ToolCtx {
    ToolCtx {
        task_id: Some("task-one".into()),
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}

#[tokio::test]
async fn command_and_agent_share_tool_permissions_without_recursively_taking_the_plugin_gate() {
    let root = std::env::temp_dir().join(format!("qzone-host-{}", rand::random::<u64>()));
    std::fs::create_dir_all(root.join("plugins/qzone")).unwrap();
    let db_path = root.join("yuantuan.db");
    let mut conn = db::connect(&db_path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    let settings = PluginSettings {
        enabled: true,
        max_concurrency: 1,
        allowed_chats: Some(vec!["42".into()]),
        config: json!({}),
        ..Default::default()
    };
    std::fs::write(
        root.join("plugins/qzone/settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let tools = Registry::new();
    let skills = SkillRegistry::new();
    let external = Arc::new(Connector::default());
    let manager = PluginManager::load_with_services(
        vec![yuantuan_plugin_qq::qzone_plugin()],
        root.clone(),
        root.join("source"),
        tools.clone(),
        skills.clone(),
        Arc::new(RwLock::new(None)),
        None,
        Some(external.clone()),
    )
    .await
    .unwrap();
    let tool = tools.get("qzone.publish").unwrap();
    yuantuan_core::capabilities::register(
        &db_path,
        "qzone.publish",
        "tool",
        tool.description(),
        &tool.parameters_schema(),
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE capability_catalog SET enabled=0 WHERE name='qzone.publish' AND kind='tool'",
        params![],
    )
    .await
    .unwrap();
    let skill = skills.get("qzone.publish").unwrap();
    assert!(!skill.accepts_empty_command());
    assert!(skills.get("qzone.status").unwrap().accepts_empty_command());
    assert!(skills.get("qzone.history").unwrap().accepts_empty_command());
    assert!(skill
        .invoke(&context(), json!({"content":"禁用Tool时不应发布"}))
        .await
        .is_err());
    assert!(external.published.lock().unwrap().is_empty());
    conn.execute("UPDATE capability_catalog SET enabled=1,scopes='[\"private:123\"]' WHERE name='qzone.publish' AND kind='tool'",params![]).await.unwrap();
    assert!(skill
        .invoke(&context(), json!({"content":"范围不符不发布"}))
        .await
        .is_err());
    conn.execute("UPDATE capability_catalog SET scopes='[\"group:42\"]' WHERE name='qzone.publish' AND kind='tool'",params![]).await.unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        skill.invoke(
            &context(),
            json!({"explicit_command":true,"user_text":"--好友 已完成连接"}),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.data["receipt"]["tid"], "verified-id");
    assert_eq!(external.published.lock().unwrap().len(), 1);
    assert_eq!(
        external.published.lock().unwrap()[0].visibility,
        QzoneVisibility::Friends
    );
    let other = ToolCtx {
        chat_id: "43".into(),
        ..context()
    };
    assert!(tool
        .call(&other, json!({"content":"不在插件范围"}))
        .await
        .is_err());
    let other = ToolCtx {
        sender_pid: "p_456".into(),
        ..context()
    };
    assert!(tool
        .call(&other, json!({"content":"未经授权"}))
        .await
        .is_err());
    assert_eq!(external.published.lock().unwrap().len(), 1);
    manager.shutdown().await;
    drop(conn);
    let _ = std::fs::remove_dir_all(root);
}
