use super::*;
use anyhow::bail;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct Host {
    data: Mutex<Option<Value>>,
    posts: Mutex<Vec<QzonePost>>,
    enabled: AtomicBool,
    mode: AtomicU8,
    saves: AtomicUsize,
}
impl Host {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(true),
            ..Default::default()
        })
    }
}
#[async_trait]
impl HostServices for Host {
    async fn qzone(&self, ctx: &ToolCtx, action: QzoneAction) -> Result<Value> {
        ensure!(
            ctx.sender_pid == "p_123",
            ToolError::new(ToolErrorKind::PermissionDenied, "非超级管理员")
        );
        match action {
            QzoneAction::CheckAccess { .. } | QzoneAction::Status => {
                Ok(json!({"qq":999,"connected":true}))
            }
            QzoneAction::Publish { post, account_qq } => {
                assert_eq!(account_qq, 999);
                self.posts.lock().unwrap().push(post);
                match self.mode.load(Ordering::SeqCst) {
                    1 => Err(ToolError::new(ToolErrorKind::ResultUnknown, "模拟未确认").into()),
                    2 => Ok(json!({"ok":true})),
                    5 => std::future::pending().await,
                    _ => Ok(json!({"status":"published","tid":"real-tid","qq":999})),
                }
            }
        }
    }
    async fn authorize_tool_for_skill(
        &self,
        _ctx: &ToolCtx,
        name: &str,
        _args: &Value,
    ) -> Result<()> {
        ensure!(
            name.starts_with("qzone.") && self.enabled.load(Ordering::SeqCst),
            "Tool已停用"
        );
        Ok(())
    }
    async fn chat(&self, _: &ToolCtx, _: &str, _: &str) -> Result<String> {
        bail!("不应调用模型")
    }
    async fn read_data(&self, _: &str) -> Result<Option<Value>> {
        Ok(self.data.lock().unwrap().clone())
    }
    async fn write_data(&self, _: &str, value: Value) -> Result<()> {
        let save = self.saves.fetch_add(1, Ordering::SeqCst);
        let mode = self.mode.load(Ordering::SeqCst);
        ensure!(mode != 3 && !(mode == 4 && save > 0), "模拟记录保存失败");
        *self.data.lock().unwrap() = Some(value);
        Ok(())
    }
    fn log(&self, _: &str) {}
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}
async fn load_caps(host: Arc<Host>, limit: u64) -> Capabilities {
    plugin()
        .start(PluginContext { host }, json!({"daily_limit":limit}))
        .await
        .unwrap()
}

#[tokio::test]
async fn commands_skills_and_tools_share_one_publisher_and_no_model() {
    let host = Host::new();
    let caps = load_caps(host.clone(), 10).await;
    let skill = &caps.skills[0];
    let tool = &caps.tools[0];
    let context = ctx();
    let (a, b) = tokio::join!(
        skill.invoke(
            &context,
            json!({"explicit_command":true,"user_text":"今天完成了测试"})
        ),
        tool.call(&context, json!({"content":"今天完成了测试"}))
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(host.posts.lock().unwrap().len(), 1);
    assert_eq!(
        host.posts.lock().unwrap()[0].visibility,
        QzoneVisibility::Friends
    );
    let output = skill
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"--私密 私人的记录"}),
        )
        .await
        .unwrap();
    assert_eq!(output.data["receipt"]["tid"], "real-tid");
    assert_eq!(
        host.posts.lock().unwrap()[1].visibility,
        QzoneVisibility::Private
    );
    let history = caps.tools[2].call(&ctx(), json!({})).await.unwrap();
    assert_eq!(
        history.data["records"][0]["preview"],
        "私密说说（正文隐藏）"
    );
    let private = ToolCtx {
        chat_type: "private".into(),
        ..ctx()
    };
    assert_eq!(
        caps.tools[2].call(&private, json!({})).await.unwrap().data["records"][0]["preview"],
        "私人的记录"
    );
    assert_eq!(tool.policy().effect, ToolEffect::ExternalWrite);
}

#[tokio::test]
async fn disabled_tools_bad_parameters_and_non_owners_never_publish() {
    let host = Host::new();
    let caps = load_caps(host.clone(), 10).await;
    host.enabled.store(false, Ordering::SeqCst);
    assert!(caps.skills[0]
        .invoke(&ctx(), json!({"content":"不应发布"}))
        .await
        .is_err());
    host.enabled.store(true, Ordering::SeqCst);
    for args in [
        json!({"content":"  "}),
        json!({"content":"x","images":["https://example.com/a.png"]}),
        json!({"content":"x","visibility":"wrong"}),
        json!({"content":"x","actor":"p_123"}),
        json!({"content":"长".repeat(2001)}),
    ] {
        assert!(caps.tools[0].call(&ctx(), args).await.is_err());
    }
    let mut other = ctx();
    other.sender_pid = "p_456".into();
    assert!(caps.tools[0]
        .call(&other, json!({"content":"他人的请求"}))
        .await
        .is_err());
    assert_eq!(host.posts.lock().unwrap().len(), 0);
    assert_eq!(host.saves.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn uncertain_results_persist_across_reload_and_cannot_be_replayed_with_new_tasks() {
    let host = Host::new();
    host.mode.store(1, Ordering::SeqCst);
    let first = load_caps(host.clone(), 10).await;
    assert!(first.tools[0]
        .call(&ctx(), json!({"content":"结果待核实"}))
        .await
        .is_err());
    host.mode.store(0, Ordering::SeqCst);
    let reloaded = load_caps(host.clone(), 10).await;
    let mut other_task = ctx();
    other_task.task_id = Some("another-task".into());
    assert!(reloaded.tools[0]
        .call(&other_task, json!({"content":"结果待核实"}))
        .await
        .is_err());
    assert_eq!(host.posts.lock().unwrap().len(), 1);
    let history = reloaded.tools[2].call(&ctx(), json!({})).await.unwrap();
    assert_eq!(history.data["records"][0]["status"], "unknown");
}

#[tokio::test]
async fn quota_is_durable_and_a_missing_receipt_is_not_success() {
    let host = Host::new();
    let first = load_caps(host.clone(), 1).await;
    first.tools[0]
        .call(&ctx(), json!({"content":"第一条"}))
        .await
        .unwrap();
    let reloaded = load_caps(host.clone(), 1).await;
    assert!(reloaded.tools[0]
        .call(&ctx(), json!({"content":"超出额度"}))
        .await
        .is_err());
    assert_eq!(host.posts.lock().unwrap().len(), 1);
    let missing = Host::new();
    missing.mode.store(2, Ordering::SeqCst);
    let caps = load_caps(missing.clone(), 10).await;
    let error = caps.tools[0]
        .call(&ctx(), json!({"content":"无真实tid"}))
        .await
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<ToolError>().unwrap().kind,
        ToolErrorKind::ResultUnknown
    );
}

#[tokio::test]
async fn a_failed_reservation_never_publishes_and_a_failed_receipt_save_never_invites_retry() {
    let host = Host::new();
    host.mode.store(3, Ordering::SeqCst);
    let caps = load_caps(host.clone(), 10).await;
    assert!(caps.tools[0]
        .call(&ctx(), json!({"content":"存不下预留记录"}))
        .await
        .is_err());
    assert!(host.posts.lock().unwrap().is_empty());
    let host = Host::new();
    host.mode.store(4, Ordering::SeqCst);
    let caps = load_caps(host.clone(), 10).await;
    let output = caps.tools[0]
        .call(&ctx(), json!({"content":"已发布但保存回执失败"}))
        .await
        .unwrap();
    assert_eq!(output.data["receipt_recorded"], false);
    assert_eq!(output.data["receipt"]["tid"], "real-tid");
    assert!(caps.tools[0]
        .call(&ctx(), json!({"content":"已发布但保存回执失败"}))
        .await
        .is_err());
    assert_eq!(host.posts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancelled_publish_remains_uncertain_after_restart() {
    let host = Host::new();
    host.mode.store(5, Ordering::SeqCst);
    let caps = load_caps(host.clone(), 10).await;
    let tool = caps.tools[0].clone();
    let task = tokio::spawn(async move { tool.call(&ctx(), json!({"content":"调用中断"})).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while host.posts.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    host.mode.store(0, Ordering::SeqCst);
    let reloaded = load_caps(host.clone(), 10).await;
    assert!(reloaded.tools[0]
        .call(&ctx(), json!({"content":"调用中断"}))
        .await
        .is_err());
    assert_eq!(host.posts.lock().unwrap().len(), 1);
}
