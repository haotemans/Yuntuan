use anyhow::Result;

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn native_agent_queries_current_revision_and_applies_a_real_guarded_plugin_patch(){
    use yuantuan_core::{agent::{self,TaskRunnerDeps},event::{EventBus,Event},llm::LlmGateway};
    use axum::{Router,routing::post,Json};
    let (path,p,m,_r,_)=setup(Some(123),false).await;
    let tools=Registry::new();
    tools.register(ManagementTool{service:m.clone(),write:false});tools.register(ManagementTool{service:m.clone(),write:true});
    let calls=Arc::new(AtomicUsize::new(0));let seen=calls.clone();let host=p.clone();
    let app=Router::new().route("/chat/completions",post(move|Json(body):Json<Value>|{
        let seen=seen.clone();let host=host.clone();async move {
            let n=seen.fetch_add(1,Ordering::SeqCst);
            assert!(!body.to_string().contains("private-credential"));
            let (name,args)=match n {
                0=>(agent::protocol::alias("management.plugins_status"),json!({})),
                1=>{
                    let revision=host.list()[0]["revision"].clone();
                    assert!(body.to_string().contains(revision.as_str().unwrap()));
                    (agent::protocol::alias("management.plugins_apply"),json!({"plugin":"fixture","operation":"configure","revision":revision,"patch":{"count":5}}))
                },
                _=>("yuantuan_control".into(),json!({"action":"done","text":"结果数已改为5并生效"})),
            };
            Json(json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":format!("call_{n}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}))
        }
    }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap()});
    let providers=path.parent().unwrap().join("mock-providers.toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url=\"http://{addr}\"\napi_key_env=\"\"\n[roles]\nagent_exec={{provider=\"mock\",model=\"m\",tool_protocol=\"native\"}}\n")).unwrap();
    let bus=EventBus::new(32);let mut events=bus.subscribe();
    let runner=agent::spawn_runner(TaskRunnerDeps{db_path:path.clone(),tools:tools.clone(),bus:bus.clone(),llm:Arc::new(RwLock::new(Some(Arc::new(LlmGateway::load(&providers).unwrap()))))});
    let id=agent::create_scoped_task(&path,&bus,"42","private","p_123",None,"把测试插件结果数改成5",Default::default(),json!({})).await.unwrap();
    let finished=tokio::time::timeout(std::time::Duration::from_secs(10),async {loop{if let Event::TaskFinished(e)=events.recv().await.unwrap(){if e.task_id==id {break e}}}}).await.unwrap();
    assert_eq!(finished.final_state.as_deref(),Some("finished"));assert_eq!(calls.load(Ordering::SeqCst),3);
    assert_eq!(m.status(&Caller::Chat(context("p_123"))).unwrap()[0]["config"]["count"],5);
    assert!(m.history().await.unwrap().iter().any(|v|v["record"]["actor"]=="p_123"&&v["record"]["status"]=="applied"));
    runner.abort();let _=runner.await;server.abort();p.shutdown().await;
}
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
};
use yuantuan_core::{
    db,
    management::{Action, Caller, Grant, Management, ManagementTool, RestartSkill},
    plugins::{PluginManager, PluginSettings},
    skills::SkillRegistry,
    tools::Registry,
};
use yuantuan_plugin_api::*;

struct Fixture {
    starts: Arc<AtomicUsize>,
}
#[async_trait]
impl Plugin for Fixture {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "fixture",
            name: "测试插件",
            version: "1",
            description: "fixture",
            api_version: API_VERSION,
        }
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "count",
                label: "结果数",
                kind: ConfigKind::Integer,
                default: json!(3),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "key",
                label: "密钥",
                kind: ConfigKind::String,
                default: json!(""),
                secret: true,
                options: vec![],
            },
            ConfigField {
                key: "api_url",
                label: "地址",
                kind: ConfigKind::String,
                default: json!("https://example.org"),
                secret: false,
                options: vec![],
            },
        ]
    }
    async fn start(&self, _: PluginContext, _: Value) -> Result<Capabilities> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Capabilities::default())
    }
}
fn context(actor: &str) -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "42".into(),
        chat_type: "private".into(),
        sender_pid: actor.into(),
        locale: None,
    }
}
async fn setup(
    owner: Option<u64>,
    restart: bool,
) -> (
    PathBuf,
    Arc<PluginManager>,
    Arc<Management>,
    tokio::sync::watch::Receiver<bool>,
    Arc<AtomicUsize>,
) {
    let root = std::env::temp_dir().join(format!("yt-management-{}", rand::random::<u64>()));
    std::fs::create_dir_all(root.join("plugins/fixture")).unwrap();
    let path = root.join("yuantuan.db");
    let mut c = db::connect(&path).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    let settings = PluginSettings {
        enabled: true,
        allowed_chats: Some(vec!["42".into()]),
        timeout_ms: 12345,
        config: json!({"count":3,"key":"private-credential","api_url":"https://example.org"}),
        ..Default::default()
    };
    std::fs::write(
        root.join("plugins/fixture/settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let plugins = PluginManager::load(
        vec![Arc::new(Fixture {
            starts: starts.clone(),
        })],
        root.clone(),
        root.join("source"),
        Registry::new(),
        SkillRegistry::new(),
        Arc::new(RwLock::new(None)),
    )
    .await
    .unwrap();
    let (m, r) = Management::new(path.clone(), plugins.clone(), owner, restart)
        .await
        .unwrap();
    (path, plugins, m, r, starts)
}
fn action(p: &PluginManager, operation: &str, patch: Value) -> Action {
    Action {
        plugin: "fixture".into(),
        operation: operation.into(),
        revision: p.list()[0]["revision"].as_str().unwrap().into(),
        patch: patch.as_object().unwrap().clone(),
    }
}

#[tokio::test]
async fn ordinary_group_official_and_scheduled_identities_cannot_inherit_owner_permissions() {
    let (_path, p, m, _r, _) = setup(Some(123), false).await;
    let mut cases = vec![
        context("p_456"),
        context("qq-official_user_app_openid"),
        context("scheduler"),
    ];
    let mut group = context("p_123");
    group.chat_type = "group".into();
    cases.push(group);
    let mut scheduled = context("p_123");
    scheduled.chat_id = "scheduled".into();
    cases.push(scheduled);
    for c in cases {
        assert!(m.status(&Caller::Chat(c.clone())).is_err());
        assert!(m
            .apply(&Caller::Chat(c), action(&p, "disable", json!({})))
            .await
            .is_err());
    }
    assert_eq!(p.list()[0]["enabled"], true);
    let tool = ManagementTool {
        service: m.clone(),
        write: true,
    };
    let mut c = context("p_123");
    c.task_id = Some("foreign".into());
    assert!(tool
        .call(
            &c,
            json!({"plugin":"fixture","operation":"disable","revision":p.list()[0]["revision"]})
        )
        .await
        .is_err());
    let (_, _, unconfigured, _, _) = setup(None, false).await;
    assert!(!unconfigured.visible(&context("p_123")));
}

#[tokio::test]
async fn owner_and_scoped_admin_use_same_reload_boundary_preserving_credentials_and_rejecting_stale_writes(
) {
    let (path, p, m, _r, starts) = setup(Some(123), false).await;
    let owner = Caller::Chat(context("p_123"));
    let before = m.status(&owner).unwrap();
    assert!(!serde_json::to_string(&before)
        .unwrap()
        .contains("private-credential"));
    assert!(!before[0]["config"]
        .as_object()
        .unwrap()
        .contains_key("api_url"));
    let stale = action(&p, "configure", json!({"count":7}));
    let applied = m
        .apply(&owner, action(&p, "configure", json!({"count":5})))
        .await
        .unwrap();
    assert_eq!(applied["status"], "applied");
    assert!(m.apply(&owner, stale).await.is_err());
    assert!(m
        .apply(&owner, action(&p, "configure", json!({"key":"replace"})))
        .await
        .is_err());
    assert!(m
        .apply(
            &owner,
            action(&p, "configure", json!({"api_url":"https://evil.example"}))
        )
        .await
        .is_err());
    let saved: PluginSettings = serde_json::from_slice(
        &std::fs::read(path.parent().unwrap().join("plugins/fixture/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.config["key"], "private-credential");
    assert_eq!(saved.config["count"], 5);
    assert_eq!(saved.timeout_ms, 12345);
    assert_eq!(saved.allowed_chats, Some(vec!["42".into()]));
    m.set_grants(vec![Grant {
        qq: "456".into(),
        plugins: vec!["fixture".into()],
        configure: true,
        toggle: false,
        reload: true,
    }])
    .await
    .unwrap();
    let admin = Caller::Chat(context("p_456"));
    assert!(m.status(&admin).is_ok());
    assert!(m
        .apply(&admin, action(&p, "disable", json!({})))
        .await
        .is_err());
    let count = starts.load(Ordering::SeqCst);
    m.apply(&admin, action(&p, "reload", json!({})))
        .await
        .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), count + 1);
    let (restored, _) = Management::new(path, p.clone(), Some(123), false)
        .await
        .unwrap();
    assert!(restored.visible(&context("p_456")));
    m.set_grants(vec![]).await.unwrap();
    assert!(m
        .apply(&admin, action(&p, "configure", json!({"count":9})))
        .await
        .is_err());
    let history = m.history().await.unwrap();
    assert!(history.iter().any(|v| v["record"]["status"] == "denied"));
    assert!(!serde_json::to_string(&history).unwrap().contains("private-credential"));
}

#[tokio::test]
async fn only_owner_explicit_restart_reaches_supervisor_channel_once() {
    let (_, _, m, mut restart, _) = setup(Some(123), true).await;
    let skill = RestartSkill(m.clone());
    assert!(skill
        .invoke(
            &context("p_123"),
            json!({"explicit_command":true,"original_user_text":"请总结这篇网页"})
        )
        .await
        .is_err());
    assert!(skill
        .invoke(
            &context("p_456"),
            json!({"explicit_command":true,"original_user_text":"/重启云团"})
        )
        .await
        .is_err());
    assert!(!*restart.borrow());
    skill
        .invoke(
            &context("p_123"),
            json!({"explicit_command":true,"original_user_text":"/重启云团"}),
        )
        .await
        .unwrap();
    restart.changed().await.unwrap();
    assert!(*restart.borrow());
    assert!(m.request_restart(&Caller::Webui).await.is_err());
    let (_, _, disabled, _r, _) = setup(Some(123), false).await;
    assert!(disabled.request_restart(&Caller::Webui).await.is_err());
}
