//! Protocol parity, durable execution and failure boundaries, using local mocks only.
use anyhow::Result;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use yuantuan_core::db::SqliteExt;
use yuantuan_core::{
    agent::{self, protocol, runtime},
    db,
    event::{Event, EventBus},
    llm::LlmGateway,
    tools::*,
};

struct Fixture(PathBuf);
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yt-tool-protocol-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let f = Self(dir);
        let mut c = db::connect(&f.db()).await.unwrap();
        db::migrate(&mut c).await.unwrap();
        f
    }
    fn db(&self) -> PathBuf {
        self.0.join("test.db")
    }
    async fn task(&self) -> (String, runtime::Checkpoint, ToolCtx) {
        let id = agent::create_task(
            &self.db(),
            &EventBus::default(),
            "42",
            "p_owner",
            "check tools",
        )
        .await
        .unwrap();
        let cp = runtime::load(&self.db(), &id).await.unwrap().unwrap();
        runtime::reserve(&self.db(), &id, &cp, 1).await.unwrap();
        let ctx = ToolCtx {
            task_id: Some(id.clone()),
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_owner".into(),
            locale: None,
        };
        (id, cp, ctx)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Probe {
    calls: Arc<AtomicUsize>,
    behavior: &'static str,
    effect: ToolEffect,
}
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &'static str {
        "mcp:probe.lookup"
    }
    fn description(&self) -> &'static str {
        "local test tool"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","required":["q"],"properties":{"q":{"type":"integer"}}})
    }
    fn policy(&self) -> ToolPolicy {
        ToolPolicy {
            effect: self.effect,
            timeout_ms: if self.behavior == "long" { 5000 } else { 25 },
            cancellation: ToolCancellation::DropFuture,
            ..Default::default()
        }
    }
    async fn call(&self, _: &ToolCtx, args: Value) -> Result<ToolOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            "timeout" | "long" => tokio::time::sleep(Duration::from_secs(30)).await,
            "panic" => panic!("test panic"),
            "failure" => anyhow::bail!("transport failed after submission"),
            "rejected" => {
                return Err(ToolError::new(
                    ToolErrorKind::InvalidArguments,
                    "invalid before effect",
                )
                .into())
            }
            _ => {}
        }
        Ok(ToolOutput {
            summary: "verified result".into(),
            artifacts: vec![],
            data: args,
        })
    }
}
fn registry(behavior: &'static str, effect: ToolEffect) -> (Registry, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let r = Registry::new();
    r.register(Probe {
        calls: calls.clone(),
        behavior,
        effect,
    });
    (r, calls)
}
fn call(q: Value) -> ToolCall {
    ToolCall {
        call_id: "original_1".into(),
        name: "mcp:probe.lookup".into(),
        arguments: q,
    }
}
fn response(name: &str, args: &str) -> Value {
    json!({"choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"original_1","type":"function","function":{"name":name,"arguments":args}}]}}]})
}

#[tokio::test]
async fn preconditions_have_no_effect_and_registration_preserves_switches_and_scopes() {
    let f = Fixture::new().await;
    let (id, mut cp, ctx) = f.task().await;
    let (r, calls) = registry("ok", ToolEffect::ReadOnly);
    let catalog = executor::catalog(&f.db(), &r, &ctx, &[]).await.unwrap();
    assert_eq!(catalog.len(), 1);
    let rejected = executor::execute(
        &f.db(),
        &r,
        &ctx,
        &cp,
        call(json!({"q":"bad"})),
        ToolProtocol::Native,
    )
    .await
    .unwrap();
    assert_eq!(rejected.result.status, ToolStatus::Rejected);
    assert_eq!(
        rejected.result.error.as_ref().unwrap().kind,
        ToolErrorKind::InvalidArguments
    );
    assert!(rejected
        .result
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("/q"));
    assert!(!rejected.result.external_action_started);
    assert!(rejected.result.receipt_id.is_none());
    assert!(rejected.result.retryable);
    cp.history.push(rejected.history_record());
    executor::commit(&f.db(), &id, &cp, &rejected)
        .await
        .unwrap();
    let mut c = db::connect(&f.db()).await.unwrap();
    c.execute_batch("UPDATE capability_catalog SET scopes='[\"private:42\"]';")
        .await
        .unwrap();
    assert!(executor::catalog(&f.db(), &r, &ctx, &[])
        .await
        .unwrap()
        .is_empty());
    executor::register(&f.db(), r.get("mcp:probe.lookup").unwrap().as_ref())
        .await
        .unwrap();
    let denied = executor::execute(
        &f.db(),
        &r,
        &ctx,
        &cp,
        call(json!({"q":1})),
        ToolProtocol::Json,
    )
    .await
    .unwrap();
    assert_eq!(denied.result.status, ToolStatus::Rejected);
    c.execute_batch("UPDATE capability_catalog SET enabled=0,scopes='[]'")
        .await
        .unwrap();
    assert!(executor::catalog(&f.db(), &r, &ctx, &[])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_invocations")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let scopes: String = sqlx::query_scalar("SELECT scopes FROM capability_catalog")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(scopes, "[]");
    let disabled = Registry::new();
    disabled
        .register(yuantuan_core::tools::sandbox::SandboxExecTool::new(Default::default()).unwrap());
    assert!(executor::catalog(&f.db(), &disabled, &ctx, &[])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn durable_receipts_reject_replay_foreign_context_and_duplicate_completion() {
    let f = Fixture::new().await;
    let (id, mut cp, ctx) = f.task().await;
    let (r, calls) = registry("ok", ToolEffect::ReadOnly);
    let mut foreign = ctx.clone();
    foreign.chat_type = "private".into();
    assert!(executor::execute(
        &f.db(),
        &r,
        &foreign,
        &cp,
        call(json!({"q":1})),
        ToolProtocol::Native
    )
    .await
    .is_err());
    foreign = ctx.clone();
    foreign.chat_id = "other".into();
    assert!(executor::execute(
        &f.db(),
        &r,
        &foreign,
        &cp,
        call(json!({"q":1})),
        ToolProtocol::Native
    )
    .await
    .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let out = executor::execute(
        &f.db(),
        &r,
        &ctx,
        &cp,
        call(json!({"q":1})),
        ToolProtocol::Native,
    )
    .await
    .unwrap();
    cp.history.push(out.history_record());
    executor::commit(&f.db(), &id, &cp, &out).await.unwrap();
    assert!(executor::commit(&f.db(), &id, &cp, &out).await.is_err());
    assert!(executor::execute(
        &f.db(),
        &r,
        &ctx,
        &cp,
        call(json!({"q":1})),
        ToolProtocol::Native
    )
    .await
    .is_err());
    let mut changed = call(json!({"q":2}));
    changed.call_id = "original_1".into();
    assert!(
        executor::execute(&f.db(), &r, &ctx, &cp, changed, ToolProtocol::Native)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let wire = protocol::transcript(&cp.history).unwrap();
    assert_eq!(wire.len(), 3);
    assert_eq!(wire[1]["tool_calls"][0]["id"], "original_1");
    assert_eq!(wire[2]["tool_call_id"], "original_1");
    assert_eq!(
        serde_json::from_str::<Value>(wire[2]["content"].as_str().unwrap()).unwrap()["status"],
        "succeeded"
    );
    let mut with_feedback = cp.history.clone();
    with_feedback.push(json!({"role":"user","content":"latest repair instruction"}));
    let extended = protocol::transcript(&with_feedback).unwrap();
    assert_eq!(&extended[..wire.len()], wire.as_slice());
    assert_eq!(
        extended.last().unwrap()["content"],
        "latest repair instruction"
    );
    let mut corrupted = cp.history.clone();
    corrupted.last_mut().unwrap()["result"]["call_id"] = json!("wrong");
    assert!(protocol::transcript(&corrupted).is_err());
    let mut c = db::connect(&f.db()).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT result FROM task_invocations")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap()["data"]["q"],
        1
    );
    // Simulate a V17 database, preserving old receipts and task briefs.
    c.execute_batch(
        "DROP TABLE tool_invocation_records; DROP TABLE tool_contracts; PRAGMA user_version=17;",
    )
    .await
    .unwrap();
    db::migrate(&mut c).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    let preserved: String = sqlx::query_scalar("SELECT result FROM task_invocations")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(stored, preserved);
    assert!(agent::brief::current(&mut c, &id)
        .await
        .unwrap()
        .is_object());
    let v: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(v, 18);
}

#[tokio::test]
async fn uncertain_writes_stop_and_read_only_failures_are_distinguished() {
    for (behavior, effect, status, kind) in [
        (
            "timeout",
            ToolEffect::ExternalWrite,
            ToolStatus::Unknown,
            ToolErrorKind::Timeout,
        ),
        (
            "panic",
            ToolEffect::Unknown,
            ToolStatus::Unknown,
            ToolErrorKind::Panic,
        ),
        (
            "failure",
            ToolEffect::ExternalWrite,
            ToolStatus::Unknown,
            ToolErrorKind::ExecutionFailed,
        ),
        (
            "timeout",
            ToolEffect::ReadOnly,
            ToolStatus::Failed,
            ToolErrorKind::Timeout,
        ),
        (
            "rejected",
            ToolEffect::Unknown,
            ToolStatus::Rejected,
            ToolErrorKind::InvalidArguments,
        ),
    ] {
        let f = Fixture::new().await;
        let (id, mut cp, ctx) = f.task().await;
        let (r, calls) = registry(behavior, effect);
        let out = executor::execute(
            &f.db(),
            &r,
            &ctx,
            &cp,
            call(json!({"q":1})),
            ToolProtocol::Json,
        )
        .await
        .unwrap();
        assert_eq!(out.result.status, status);
        assert_eq!(out.result.error.as_ref().unwrap().kind, kind);
        cp.history.push(out.history_record());
        executor::commit(&f.db(), &id, &cp, &out).await.unwrap();
        assert_eq!(
            runtime::uncertain(&f.db(), &id).await.unwrap(),
            status == ToolStatus::Unknown
        );
        assert!(executor::execute(
            &f.db(),
            &r,
            &ctx,
            &cp,
            call(json!({"q":1})),
            ToolProtocol::Json
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn native_parser_rejects_unexecutable_envelopes_before_execution() {
    let (r, _) = registry("ok", ToolEffect::ReadOnly);
    let d = ToolDefinition::of(r.get("mcp:probe.lookup").unwrap().as_ref());
    let name = protocol::alias(&d.name);
    let catalog = vec![d];
    assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    assert!(name.len() <= 64);
    assert_eq!(
        protocol::native(&response(&name, "{\"q\":1}"), &catalog)
            .unwrap()
            .tool_name
            .as_deref(),
        Some("mcp:probe.lookup")
    );
    let mut batch = response(&name, "{}");
    let c = batch["choices"][0]["message"]["tool_calls"][0].clone();
    batch["choices"][0]["message"]["tool_calls"]
        .as_array_mut()
        .unwrap()
        .push(c);
    let mut missing = response(&name, "{}");
    missing["choices"][0]["message"]["tool_calls"][0]["id"] = Value::Null;
    for v in [
        batch,
        missing,
        response("undeclared", "{}"),
        response(&name, "[1]"),
        response(&name, "{"),
        response(&name, &"x".repeat(33000)),
        json!({"choices":[{"finish_reason":"stop","message":{"function_call":{"name":name}}}]}),
    ] {
        assert!(protocol::native(&v, &catalog).is_err());
    }
    for finish in ["length", "content_filter"] {
        let mut v = response(&name, "{}");
        v["choices"][0]["finish_reason"] = json!(finish);
        assert!(!protocol::native(&v, &catalog).unwrap_err().repairable);
    }
    let mut v = response(&name, "{}");
    v["choices"][0]["message"]["refusal"] = json!("refused");
    assert!(!protocol::native(&v, &catalog).unwrap_err().repairable);
    assert!(protocol::native(
        &response(
            protocol::CONTROL,
            r#"{"action":"tool_call","tool_name":"x"}"#
        ),
        &catalog
    )
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_and_native_loops_share_execution_and_correlated_results() {
    for mode in [ToolProtocol::Json, ToolProtocol::Native] {
        use axum::{routing::post, Json, Router};
        let f = Fixture::new().await;
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let requests = seen.clone();
        let name = protocol::alias("mcp:probe.lookup");
        let app=Router::new().route("/chat/completions",post(move |Json(body):Json<Value>|{let seen=requests.clone();let name=name.clone();async move {
            let mut seen=seen.lock().unwrap();let n=seen.len();seen.push(body);
            if mode==ToolProtocol::Native && n==0 {Json(response(&name,r#"{"q":7}"#))} else {
                let text=if n==0 {json!({"action":"tool_call","tool_name":"mcp:probe.lookup","tool_args":{"q":7}}).to_string()}else if mode==ToolProtocol::Native {"verified completion".into()}else {json!({"action":"done","text":"verified completion"}).to_string()};
                Json(json!({"choices":[{"finish_reason":"stop","message":{"content":text}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
            }
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let prov = f.0.join("providers.toml");
        std::fs::write(&prov,format!("[provider.mock]\nbase_url=\"{url}\"\n[roles]\nagent_exec={{provider=\"mock\",model=\"same-model\",tool_protocol=\"{}\"}}",if mode==ToolProtocol::Native {"native"}else{"json"})).unwrap();
        let gw = Arc::new(LlmGateway::load(&prov).unwrap());
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let (tools, calls) = registry("ok", ToolEffect::ReadOnly);
        let runner = agent::spawn_runner(agent::TaskRunnerDeps {
            db_path: f.db(),
            llm: Arc::new(std::sync::RwLock::new(Some(gw))),
            tools,
            bus: bus.clone(),
        });
        let id = agent::create_task(&f.db(), &bus, "42", "p_owner", "verify both protocols")
            .await
            .unwrap();
        let done = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Event::TaskFinished(p) = rx.recv().await.unwrap() {
                    if p.task_id == id {
                        break p;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            done.final_state.as_deref(),
            Some("finished"),
            "{:?}",
            done.error
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        let history = runtime::load(&f.db(), &id).await.unwrap().unwrap().history;
        let result = history.iter().find(|v| v["role"] == "tool_result").unwrap();
        assert_eq!(result["result"]["output"]["data"]["q"], 7);
        assert!(result["result"]["receipt_id"].is_string());
        if mode == ToolProtocol::Native {
            assert_eq!(
                requests[0]["messages"][1], requests[1]["messages"][1],
                "stable task brief prefix"
            );
            assert_eq!(
                requests[0]["messages"][2], requests[1]["messages"][2],
                "append-only execution prefix"
            );
            assert_eq!(requests[0]["parallel_tool_calls"], false);
            assert_eq!(requests[0]["tool_choice"], "auto");
            assert!(requests[0].get("response_format").is_none());
            assert_eq!(
                requests[1]["messages"][3]["tool_calls"][0]["id"],
                "original_1"
            );
            assert_eq!(requests[1]["messages"][4]["tool_call_id"], "original_1");
        } else {
            assert_eq!(requests[0]["tool_choice"], "none");
            assert_eq!(requests[0]["response_format"]["type"], "json_object");
        }
        runner.abort();
        let _ = runner.await;
        server.abort();
    }
}

#[tokio::test]
async fn cancelling_a_read_only_call_preserves_old_receipt_without_advancing_task() {
    let f = Fixture::new().await;
    let (id, mut cp, ctx) = f.task().await;
    let (r, calls) = registry("long", ToolEffect::ReadOnly);
    let (path, call_id, old, tool_ctx, tools) =
        (f.db(), id.clone(), cp.clone(), ctx.clone(), r.clone());
    let work = tokio::spawn(async move {
        executor::execute(
            &path,
            &tools,
            &tool_ctx,
            &old,
            call(json!({"q":1})),
            ToolProtocol::Native,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    runtime::control(
        &f.db(),
        &call_id,
        "p_owner",
        "42",
        "group",
        "pause",
        "pause task",
    )
    .await
    .unwrap();
    let out = work.await.unwrap().unwrap();
    assert_eq!(out.result.status, ToolStatus::Cancelled);
    cp.history.push(out.history_record());
    assert!(executor::commit(&f.db(), &id, &cp, &out).await.is_err());
    let mut c = db::connect(&f.db()).await.unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM tool_invocation_records")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
    let current = runtime::load(&f.db(), &id).await.unwrap().unwrap();
    assert_eq!(current.revision, cp.revision + 1);
    assert!(current.history.iter().all(|r| r["role"] != "tool_result"));
}
