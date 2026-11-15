use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use yuantuan_core::{
    agent::{self, brief, runtime, TaskPlan},
    db::{self, params, SqliteExt},
    event::{EventBus, MessageReceivedPayload},
    working_context,
};

async fn fixture() -> PathBuf {
    let p = std::env::temp_dir().join(format!("yt-brief-{}.db", rand::random::<u64>()));
    let mut c = db::connect(&p).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    c.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_a',1,1),('p_b',1,1),('self',1,1)").await.unwrap();
    p
}
async fn message(p: &Path, text: &str) -> MessageReceivedPayload {
    let mut c = db::connect(p).await.unwrap();
    let ts = yuantuan_core::window::now();
    c.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,at_me) VALUES('42','group','p_a',?1,?2,1)",params![text,ts]).await.unwrap();
    MessageReceivedPayload {
        msg_id: c.last_insert_rowid().await.unwrap(),
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_a".into(),
        text: text.into(),
        ts,
        at_me: true,
        has_image: false,
        image_urls: vec![],
        reply_to: None,
        sender_bot: false,
    }
}
async fn working(p: &Path, m: &MessageReceivedPayload, v: Value) -> i64 {
    working_context::apply(p, m, &serde_json::from_value(v).unwrap())
        .await
        .unwrap()
}
async fn create(p: &Path, m: &MessageReceivedPayload, context: Value) -> String {
    agent::create_scoped_task(
        p,
        &EventBus::default(),
        &m.chat_id,
        &m.chat_type,
        &m.sender_pid,
        Some(m.msg_id),
        "比较模型并整理成表格",
        TaskPlan::default(),
        context,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn handoff_amend_restart_and_completed_task_projection_preserve_source_and_budget() {
    let p = fixture().await;
    let first = message(&p, "比较A和B，使用4090，只能本地运行").await;
    let work=working(&p,&first,json!({"goal_quote":"比较A和B","conditions":[{"key":"显卡","quote":"4090"},{"key":"环境","quote":"只能本地运行"}]})).await;
    let next = message(&p, "整理成表格").await;
    working(&p,&next,json!({"context_id":work,"expected_revision":1,"conditions":[{"key":"格式","quote":"整理成表格"}]})).await;
    let mut c = db::connect(&p).await.unwrap();
    let context = working_context::view(&mut c, &next, next.msg_id)
        .await
        .unwrap();
    let id = create(&p, &next, json!({"working_context":context})).await;
    let b = brief::current(&mut c, &id).await.unwrap();
    assert_eq!(
        b["working_context"][0]["conditions"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let mut old = runtime::load(&p, &id).await.unwrap().unwrap();
    runtime::reserve(&p, &id, &old, 100).await.unwrap();
    let receipt = runtime::begin_invocation(&p, &id, &old, "inspect", &json!({}))
        .await
        .unwrap();
    old.history
        .push(json!({"role":"tool_result","receipt_id":receipt,"content":"旧版本真实结果"}));
    runtime::complete_invocation(
        &p,
        &id,
        &old,
        &receipt,
        json!({"summary":"旧版本真实结果"}),
        true,
    )
    .await
    .unwrap();
    let amend = message(&p, "改用5090，增加来源列").await;
    working(&p,&amend,json!({"context_id":work,"expected_revision":2,"conditions":[{"key":"显卡","quote":"5090"},{"key":"列","quote":"增加来源列"}]})).await;
    runtime::control_from_message(&p, &id, "amend", &amend, Some(1), Some(work))
        .await
        .unwrap();
    runtime::control_from_message(&p, &id, "amend", &amend, Some(1), Some(work))
        .await
        .unwrap();
    assert!(runtime::save(&p, &id, &old).await.is_err());
    let mut resumed = runtime::load(&p, &id).await.unwrap().unwrap();
    assert_eq!(resumed.revision, 2);
    assert_eq!(resumed.deadline, old.deadline);
    resumed.history = vec![json!({"role":"user","content":"历史已压缩"})];
    runtime::save(&p, &id, &resumed).await.unwrap();
    drop(c);
    let mut c = db::connect(&p).await.unwrap();
    let b = brief::current(&mut c, &id).await.unwrap();
    assert_eq!(b["changes"].as_array().unwrap().len(), 1);
    assert_eq!(b["changes"][0]["source"]["msg_id"], amend.msg_id);
    let conditions = b["working_context"][0]["conditions"].as_array().unwrap();
    assert!(conditions.iter().any(|v| v["quote"] == "5090"));
    assert!(conditions.iter().any(|v| v["quote"] == "只能本地运行"));
    let used: i64 = sqlx::query_scalar("SELECT used_calls FROM tasks WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(used, 1);
    let prior: String =
        sqlx::query_scalar("SELECT body FROM task_brief_versions WHERE task_id=? AND revision=1")
            .bind(&id)
            .fetch_one(&mut *c)
            .await
            .unwrap();
    assert!(prior.contains("4090"));
    c.execute(
        "UPDATE tasks SET state='finished',finished_at=unixepoch() WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    c.execute("UPDATE task_runtime SET phase='finished',final_text='已按新要求整理',delivery_state='pending' WHERE task_id=?1",params![id]).await.unwrap();
    let question = message(&p, "刚才整理的是什么").await;
    let candidates = brief::candidates(&mut c, &question, question.msg_id)
        .await
        .unwrap();
    assert_eq!(candidates[0]["task_id"], id);
    let selected = brief::selected(&mut c, &question, question.msg_id, &id)
        .await
        .unwrap();
    assert_eq!(selected["phase"], "finished");
    assert_eq!(selected["delivery_state"], "pending");
    assert_eq!(
        selected["invocations"][0]["matches_current_revision"],
        false
    );
    assert!(
        brief::selected(&mut c, &question, next.msg_id, &id)
            .await
            .is_err(),
        "旧快照不读取后来补充的原话"
    );
    for (key, value) in [
        ("chat_type", "private"),
        ("chat_id", "other"),
        ("sender_pid", "p_b"),
    ] {
        let mut foreign = question.clone();
        match key {
            "chat_type" => foreign.chat_type = value.into(),
            "chat_id" => foreign.chat_id = value.into(),
            _ => foreign.sender_pid = value.into(),
        }
        assert!(brief::selected(&mut c, &foreign, question.msg_id, &id)
            .await
            .is_err());
        assert!(brief::candidates(&mut c, &foreign, question.msg_id)
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn invalid_sources_stale_versions_and_oversize_updates_do_not_mutate_task() {
    let p = fixture().await;
    let first = message(&p, "整理数据").await;
    let id = create(&p, &first, json!({})).await;
    let next = message(&p, "增加一列").await;
    let mut forged = next.clone();
    forged.text = "伪造要求".into();
    assert!(
        runtime::control_from_message(&p, &id, "amend", &forged, Some(1), None)
            .await
            .is_err()
    );
    assert!(
        runtime::control_from_message(&p, &id, "amend", &next, Some(9), None)
            .await
            .is_err()
    );
    assert!(
        runtime::control(&p, &id, "p_b", "42", "group", "amend", "增加列")
            .await
            .is_err()
    );
    assert!(
        runtime::control(&p, &id, "p_a", "42", "group", "amend", &"太长".repeat(3000))
            .await
            .is_err()
    );
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    assert_eq!(cp.revision, 1);
    let mut c = db::connect(&p).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_brief_versions WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let invalid = agent::create_scoped_task(
        &p,
        &EventBus::default(),
        "42",
        "private",
        "p_a",
        Some(first.msg_id),
        "错误来源",
        TaskPlan::default(),
        json!({}),
    )
    .await;
    assert!(invalid.is_err());
    let source = message(&p, "另一个明确任务").await;
    let incomplete = agent::create_scoped_task(
        &p,
        &EventBus::default(),
        "42",
        "group",
        "p_a",
        Some(source.msg_id),
        "不能遗失条件",
        TaskPlan::default(),
        json!({"focus_context_id":first.msg_id,"working_context":[]}),
    )
    .await;
    assert!(incomplete.is_err(), "已选事项缺失时不能创建缩水任务");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(n, 1, "建任务失败必须整体回滚");
}

#[tokio::test]
async fn v16_upgrade_and_legacy_tasks_keep_existing_data_without_inventing_sources() {
    let p = fixture().await;
    let m = message(&p, "原始请求").await;
    let id = create(&p, &m, json!({})).await;
    let mut c = db::connect(&p).await.unwrap();
    c.execute_batch("DROP TABLE task_brief_versions; PRAGMA user_version=16")
        .await
        .unwrap();
    db::migrate(&mut c).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    let b = brief::current(&mut c, &id).await.unwrap();
    assert_eq!(b["origin"], "legacy_task");
    assert!(b["request"].is_null());
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    assert!(!cp.history.is_empty());
    runtime::control_from_message(&p, &id, "amend", &m, Some(1), None)
        .await
        .unwrap();
    let b = brief::current(&mut c, &id).await.unwrap();
    assert_eq!(b["changes"][0]["source"]["msg_id"], m.msg_id);
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(version, 17);
}

#[tokio::test]
async fn explicit_goal_change_invalidates_children_and_new_child_inherits_current_requirements() {
    let p = fixture().await;
    let m = message(&p, "比较A和B，使用4090").await;
    let work = working(
        &p,
        &m,
        json!({"goal_quote":"比较A和B","conditions":[{"key":"显卡","quote":"4090"}]}),
    )
    .await;
    let mut c = db::connect(&p).await.unwrap();
    let view = working_context::view(&mut c, &m, m.msg_id).await.unwrap();
    let id = create(&p, &m, json!({"working_context":view})).await;
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    runtime::reserve(&p, &id, &cp, 100).await.unwrap();
    let task = yuantuan_core::event::TaskLifecyclePayload {
        task_id: id.clone(),
        chat_id: "42".into(),
        goal: "比较模型并整理成表格".into(),
        final_state: None,
        error: None,
    };
    runtime::children(&p, &task, &cp, &["收集B资料".into()])
        .await
        .unwrap();
    let old_child: String =
        sqlx::query_scalar("SELECT task_id FROM task_runtime WHERE parent_id=?")
            .bind(&id)
            .fetch_one(&mut *c)
            .await
            .unwrap();
    let m = message(&p, "改成比较A和C").await;
    working(
        &p,
        &m,
        json!({"context_id":work,"expected_revision":1,"goal_quote":"比较A和C"}),
    )
    .await;
    runtime::control_from_message(&p, &id, "amend", &m, Some(1), Some(work))
        .await
        .unwrap();
    let current = brief::current(&mut c, &id).await.unwrap();
    assert_eq!(current["goal"], "比较A和C");
    let goal: String = sqlx::query_scalar("SELECT goal FROM tasks WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(goal, "比较A和C");
    let old = brief::current(&mut c, &old_child).await.unwrap();
    assert_eq!(old["revision"], 2);
    let phase: String = sqlx::query_scalar("SELECT phase FROM task_runtime WHERE task_id=?")
        .bind(&old_child)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(phase, "cancelled");
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    runtime::reserve(&p, &id, &cp, 100).await.unwrap();
    let task = yuantuan_core::event::TaskLifecyclePayload {
        goal: "比较A和C".into(),
        ..task
    };
    runtime::children(&p, &task, &cp, &["收集C资料".into()])
        .await
        .unwrap();
    let new_child: String =
        sqlx::query_scalar("SELECT task_id FROM task_runtime WHERE parent_id=? AND phase='queued'")
            .bind(&id)
            .fetch_one(&mut *c)
            .await
            .unwrap();
    let child = brief::current(&mut c, &new_child).await.unwrap();
    assert_eq!(child["parent_goal"], "比较A和C");
    assert_eq!(child["parent_revision"], 2);
    assert_eq!(
        child["working_context"][0]["conditions"][0]["quote"],
        "4090"
    );
}

struct ProbeTool(&'static str, std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[tokio::test]
async fn pending_external_action_blocks_amend_then_confirmed_recovery_applies_saved_goal() {
    let p = fixture().await;
    let m = message(&p, "比较A和B").await;
    let work = working(&p, &m, json!({"goal_quote":"比较A和B"})).await;
    let mut c = db::connect(&p).await.unwrap();
    let view = working_context::view(&mut c, &m, m.msg_id).await.unwrap();
    let id = create(&p, &m, json!({"working_context":view})).await;
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    runtime::reserve(&p, &id, &cp, 100).await.unwrap();
    let receipt = runtime::begin_invocation(&p, &id, &cp, "inspect", &json!({}))
        .await
        .unwrap();
    let change = message(&p, "改成比较A和C").await;
    working(
        &p,
        &change,
        json!({"context_id":work,"expected_revision":1,"goal_quote":"比较A和C"}),
    )
    .await;
    assert!(
        runtime::control_from_message(&p, &id, "amend", &change, Some(1), Some(work))
            .await
            .is_err()
    );
    assert_eq!(brief::current(&mut c, &id).await.unwrap()["revision"], 1);
    runtime::complete_invocation(&p, &id, &cp, &receipt, json!({"verified":true}), true)
        .await
        .unwrap();
    let resume = message(&p, "按刚才的修改继续").await;
    runtime::control_from_message(&p, &id, "amend", &resume, Some(1), Some(work))
        .await
        .unwrap();
    assert_eq!(
        brief::current(&mut c, &id).await.unwrap()["goal"],
        "比较A和C"
    );
    let used: i64 = sqlx::query_scalar("SELECT used_calls FROM tasks WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(used, 1);
}
#[async_trait::async_trait]
impl yuantuan_core::tools::Tool for ProbeTool {
    fn name(&self) -> &'static str {
        self.0
    }
    fn description(&self) -> &'static str {
        "有参数校验的测试工具"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"q":{"type":"integer"}},"required":["q"]})
    }
    async fn call(
        &self,
        _: &yuantuan_core::tools::ToolCtx,
        args: Value,
    ) -> anyhow::Result<yuantuan_core::tools::ToolOutput> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(yuantuan_core::tools::ToolOutput {
            summary: "实际执行成功".into(),
            data: args,
            artifacts: vec![],
        })
    }
}

#[tokio::test]
async fn agent_loop_uses_current_brief_after_compaction_and_repairs_args_without_external_replay() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use yuantuan_core::{agent::TaskRunnerDeps, llm::LlmGateway, tools::Registry};
    let p = fixture().await;
    let first = message(&p, "检查资料").await;
    let id = create(&p, &first, json!({})).await;
    let change = message(&p, "增加来源列").await;
    runtime::control_from_message(&p, &id, "amend", &change, Some(1), None)
        .await
        .unwrap();
    let mut cp = runtime::load(&p, &id).await.unwrap().unwrap();
    for _ in 0..20 {
        cp.history
            .push(json!({"role":"user","content":"droppable".repeat(600)}));
    }
    runtime::save(&p, &id, &cp).await.unwrap();
    let actual = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>|{
        let counter=counter.clone();async move{
            let system=body["messages"][0]["content"].as_str().unwrap();
            assert!(system.contains("- enabled:"));
            for name in ["disabled","other_scope","outside_plan"] {assert!(!system.contains(&format!("- {name}:")));}
            let ctx:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(ctx["task_brief"]["revision"],2);
            assert_eq!(ctx["task_brief"]["changes"][0]["text"],"增加来源列");
            let n=counter.fetch_add(1,Ordering::SeqCst);
            let response=match n {
                0=>{assert!(ctx["execution_history"].as_array().unwrap().len()<=10);json!({"action":"tool_call","tool_name":"enabled","tool_args":{"q":"bad"}})},
                1=>{assert!(ctx["execution_history"].as_array().unwrap().iter().any(|v|v["role"]=="validation_error"));json!({"action":"tool_call","tool_name":"enabled","tool_args":{"q":11}})},
                _=>{let receipts=ctx["execution_history"].as_array().unwrap().iter().filter(|v|v["role"]=="tool_result").collect::<Vec<_>>();assert_eq!(receipts.len(),1);assert_eq!(receipts[0]["data"]["q"],11);assert!(receipts[0]["receipt_id"].is_string());json!({"action":"done","text":"已核对要求和实际回执"})}
            };
            axum::Json(json!({"choices":[{"message":{"content":response.to_string()},"finish_reason":"stop"}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let providers = p.with_extension("toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url=\"http://{addr}\"\n[roles]\nagent_exec={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let tools = Registry::new();
    for name in ["enabled", "disabled", "other_scope", "outside_plan"] {
        tools.register(ProbeTool(name, actual.clone()));
    }
    let mut c = db::connect(&p).await.unwrap();
    c.execute(
        "UPDATE task_runtime SET allowed_tools=?2 WHERE task_id=?1",
        params![
            id,
            json!(["enabled", "disabled", "other_scope"]).to_string()
        ],
    )
    .await
    .unwrap();
    for name in tools.names() {
        let tool = tools.get(name).unwrap();
        yuantuan_core::capabilities::register(
            &p,
            name,
            "tool",
            tool.description(),
            &tool.parameters_schema(),
        )
        .await
        .unwrap();
    }
    c.execute_batch("UPDATE capability_catalog SET enabled=0 WHERE name='disabled'; UPDATE capability_catalog SET scopes='[\"private:42\"]' WHERE name='other_scope'").await.unwrap();
    let runner = agent::spawn_runner(TaskRunnerDeps {
        db_path: p.clone(),
        llm: Arc::new(std::sync::RwLock::new(Some(Arc::new(
            LlmGateway::load(&providers).unwrap(),
        )))),
        tools,
        bus: EventBus::default(),
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let state: String = sqlx::query_scalar("SELECT state FROM tasks WHERE task_id=?")
                .bind(&id)
                .fetch_one(&mut *c)
                .await
                .unwrap();
            if state != "running" {
                assert_eq!(state, "finished");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(actual.load(Ordering::SeqCst), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_invocations WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(count, 1);
    runner.abort();
    server.abort();
}
