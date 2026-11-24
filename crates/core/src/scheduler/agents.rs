//! Administrator-created daily Agent jobs. Trigger and task creation are atomic.
use super::{daily_time, next_daily, now, BEIJING_OFFSET};
use crate::{
    agent,
    db::{self, SqliteExt},
    event::EventBus,
    tools::{Registry, ToolCtx, ToolEffect},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::path::{Path, PathBuf};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub title: String,
    pub goal: String,
    pub daily_time: String,
    pub allowed_tools: Vec<String>,
    pub max_calls: i64,
    pub timeout_secs: i64,
}

pub fn context(actor: &str) -> ToolCtx {
    ToolCtx {
        task_id: Some("schedule-preflight".into()),
        chat_id: "scheduled".into(),
        chat_type: "private".into(),
        sender_pid: actor.into(),
        locale: Some("zh-CN".into()),
    }
}

pub async fn options(path: &Path, tools: &Registry, actor: &str) -> Result<Vec<Value>> {
    let mut result = vec![];
    let mut names = tools.names();
    names.sort();
    for name in names {
        let Some(tool) = tools.get(name) else {
            continue;
        };
        let policy = tool.policy();
        // External writes need their own explicit unattended authorization flow.
        if !matches!(
            policy.effect,
            ToolEffect::ReadOnly | ToolEffect::WorkspaceWrite
        ) {
            continue;
        }
        let available = tool.available_in_chat(&context(actor))
            && crate::capabilities::allowed(path, name, "tool", &context(actor)).await?;
        result.push(
            json!({"name":name,"description":tool.description(),"available":available,
            "reason":if available { "" } else { "工具已停用或未允许 private:scheduled 会话" }}),
        );
    }
    Ok(result)
}

async fn validate_tools(
    path: &Path,
    tools: &Registry,
    actor: &str,
    selected: &[String],
) -> Result<()> {
    ensure!(
        !selected.is_empty() && selected.len() <= 16,
        "请选择 1 到 16 个允许使用的工具"
    );
    let available = options(path, tools, actor).await?;
    for name in selected {
        ensure!(
            available
                .iter()
                .any(|v| v["name"] == *name && v["available"] == true),
            "工具 {name} 不可用或未获定时任务授权"
        );
    }
    Ok(())
}

pub async fn register(path: &Path, tools: &Registry, actor: &str, draft: Draft) -> Result<i64> {
    let (hour, minute) = daily_time(&draft.daily_time)?;
    ensure!(
        !draft.title.trim().is_empty() && draft.title.chars().count() <= 100,
        "任务名称须为 1 到 100 字"
    );
    ensure!(
        !draft.goal.trim().is_empty() && draft.goal.chars().count() <= 4000,
        "任务要求须为 1 到 4000 字"
    );
    ensure!(
        (1..=10).contains(&draft.max_calls) && (30..=900).contains(&draft.timeout_secs),
        "最多 10 轮，任务时限为 30 到 900 秒"
    );
    validate_tools(path, tools, actor, &draft.allowed_tools).await?;
    let mut c = db::connect(path).await?;
    let id = sqlx::query_scalar("INSERT INTO agent_schedules(title,goal,actor_pid,allowed_tools,max_calls,timeout_secs,local_hour,local_minute,next_run_at)
        SELECT ?,?,?,?,?,?,?,?,? WHERE (SELECT COUNT(*) FROM agent_schedules)<200 RETURNING id")
        .bind(draft.title.trim()).bind(draft.goal.trim()).bind(actor).bind(serde_json::to_string(&draft.allowed_tools)?)
        .bind(draft.max_calls).bind(draft.timeout_secs).bind(hour).bind(minute).bind(next_daily(now(), BEIJING_OFFSET, hour, minute))
        .fetch_optional(&mut *c).await?.context("最多保存 200 个 Agent 定时任务")?;
    Ok(id)
}

pub async fn list(path: &Path) -> Result<Vec<Value>> {
    let mut c = db::connect(path).await?;
    let rows = sqlx::query("SELECT s.*, r.task_id,r.triggered_at,r.error,t.state,tr.phase,tr.final_text,tr.stop_reason
        FROM agent_schedules s LEFT JOIN agent_schedule_runs r ON r.id=(SELECT MAX(id) FROM agent_schedule_runs WHERE schedule_id=s.id)
        LEFT JOIN tasks t ON t.task_id=r.task_id LEFT JOIN task_runtime tr ON tr.task_id=r.task_id ORDER BY s.id DESC LIMIT 200")
        .fetch_all(&mut *c).await?;
    rows.iter().map(|r| Ok(json!({"id":r.try_get::<i64,_>("id")?,"title":r.try_get::<String,_>("title")?,"goal":r.try_get::<String,_>("goal")?,
        "allowed_tools":serde_json::from_str::<Value>(&r.try_get::<String,_>("allowed_tools")?)?,
        "max_calls":r.try_get::<i64,_>("max_calls")?,"timeout_secs":r.try_get::<i64,_>("timeout_secs")?,
        "local_hour":r.try_get::<i64,_>("local_hour")?,"local_minute":r.try_get::<i64,_>("local_minute")?,"enabled":r.try_get::<bool,_>("enabled")?,
        "next_run_at":r.try_get::<i64,_>("next_run_at")?,"task_id":r.try_get::<Option<String>,_>("task_id")?,
        "triggered_at":r.try_get::<Option<i64>,_>("triggered_at")?,"phase":r.try_get::<Option<String>,_>("phase")?,
        "result":r.try_get::<Option<String>,_>("final_text")?,"error":r.try_get::<Option<String>,_>("error")?.or(r.try_get::<Option<String>,_>("stop_reason")?)}))).collect()
}

pub async fn history(path: &Path, id: i64) -> Result<Vec<Value>> {
    let mut c = db::connect(path).await?;
    let rows = sqlx::query("SELECT r.*,tr.phase,tr.final_text,tr.stop_reason FROM agent_schedule_runs r LEFT JOIN task_runtime tr USING(task_id) WHERE schedule_id=? ORDER BY r.id DESC LIMIT 30")
        .bind(id).fetch_all(&mut *c).await?;
    rows.iter().map(|r| Ok(json!({"id":r.try_get::<i64,_>("id")?,"task_id":r.try_get::<Option<String>,_>("task_id")?,
        "triggered_at":r.try_get::<i64,_>("triggered_at")?,"phase":r.try_get::<Option<String>,_>("phase")?,
        "result":r.try_get::<Option<String>,_>("final_text")?,"error":r.try_get::<Option<String>,_>("error")?.or(r.try_get::<Option<String>,_>("stop_reason")?)}))).collect()
}

pub async fn update(
    path: &Path,
    id: i64,
    enabled: Option<bool>,
    time: Option<&str>,
    remove: bool,
) -> Result<()> {
    let mut c = db::connect(path).await?;
    let changed = if remove {
        sqlx::query("DELETE FROM agent_schedules WHERE id=?")
            .bind(id)
            .execute(&mut *c)
            .await?
            .rows_affected()
    } else if let Some(time) = time {
        let (h, m) = daily_time(time)?;
        sqlx::query(
            "UPDATE agent_schedules SET local_hour=?,local_minute=?,next_run_at=? WHERE id=?",
        )
        .bind(h)
        .bind(m)
        .bind(next_daily(now(), BEIJING_OFFSET, h, m))
        .bind(id)
        .execute(&mut *c)
        .await?
        .rows_affected()
    } else {
        sqlx::query("UPDATE agent_schedules SET enabled=? WHERE id=?")
            .bind(enabled.context("缺少启停状态")?)
            .bind(id)
            .execute(&mut *c)
            .await?
            .rows_affected()
    };
    ensure!(changed == 1, "定时任务不存在");
    Ok(())
}

pub async fn trigger(
    path: &Path,
    tools: &Registry,
    bus: &EventBus,
    id: i64,
    force: bool,
    model_ready: bool,
) -> Result<String> {
    let mut c = db::connect(path).await?;
    let row = sqlx::query("SELECT * FROM agent_schedules WHERE id=?")
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
    let actor: String = row.try_get("actor_pid")?;
    let selected: Vec<String> = serde_json::from_str(&row.try_get::<String, _>("allowed_tools")?)?;
    let preflight = async {
        ensure!(
            model_ready,
            "尚未配置 Agent 执行模型，请在模型服务中设置 agent_exec"
        );
        validate_tools(path, tools, &actor, &selected).await
    }
    .await;
    let mut tx = c.begin_immediate().await?;
    // Re-read under the write lock so pause, delete, time edits and duplicate scans win atomically.
    let current = sqlx::query("SELECT * FROM agent_schedules WHERE id=? AND enabled=1 AND (?=1 OR next_run_at<=unixepoch())
        AND NOT EXISTS(SELECT 1 FROM agent_schedule_runs r JOIN tasks t USING(task_id) WHERE r.schedule_id=agent_schedules.id AND t.state='running')")
        .bind(id).bind(force).fetch_optional(&mut *tx).await?.context("任务已暂停、未到期或上次执行尚未结束")?;
    let at = now();
    let goal: String = current.try_get("goal")?;
    let next = next_daily(
        at,
        BEIJING_OFFSET,
        current.try_get("local_hour")?,
        current.try_get("local_minute")?,
    );
    let task_id = if preflight.is_ok() {
        let task_id = agent::new_task_id();
        let plan = agent::TaskPlan {
            max_calls: current.try_get("max_calls")?,
            timeout_secs: current.try_get("timeout_secs")?,
            allowed_tools: selected,
            ..Default::default()
        };
        agent::insert_scoped_task(
            &mut tx,
            &task_id,
            "scheduled",
            "private",
            &actor,
            None,
            &goal,
            &plan,
            &json!({"schedule_id":id,"triggered_at":at}),
        )
        .await?;
        // Explicit administrator provenance; no synthetic user message or ambient chat replay.
        sqlx::query("UPDATE task_brief_versions SET body=json_set(body,'$.origin','admin_schedule','$.schedule_id',?) WHERE task_id=? AND revision=1")
            .bind(id).bind(&task_id).execute(&mut *tx).await?;
        Some(task_id)
    } else {
        None
    };
    let error = preflight.err().map(|e| e.to_string());
    sqlx::query(
        "INSERT INTO agent_schedule_runs(schedule_id,task_id,triggered_at,error) VALUES(?,?,?,?)",
    )
    .bind(id)
    .bind(&task_id)
    .bind(at)
    .bind(&error)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE agent_schedules SET next_run_at=? WHERE id=?")
        .bind(next)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(id) = task_id {
        bus.publish(crate::event::Event::TaskCreated(
            crate::event::TaskLifecyclePayload {
                task_id: id.clone(),
                chat_id: "scheduled".into(),
                goal,
                final_state: None,
                error: None,
            },
        ));
        Ok(id)
    } else {
        anyhow::bail!("{}", error.unwrap_or_default())
    }
}

pub fn spawn(
    path: PathBuf,
    tools: Registry,
    bus: EventBus,
    llm: agent::SharedLlm,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let ready = llm
                .read()
                .unwrap()
                .as_ref()
                .is_some_and(|g| g.role(crate::llm::Role::AgentExec).is_some());
            let result: Result<()> = async {
                let mut c = db::connect(&path).await?;
                let ids:Vec<i64> = sqlx::query_scalar("SELECT s.id FROM agent_schedules s WHERE enabled=1 AND next_run_at<=unixepoch()
                    AND NOT EXISTS(SELECT 1 FROM agent_schedule_runs r JOIN tasks t USING(task_id) WHERE r.schedule_id=s.id AND t.state='running') ORDER BY next_run_at,id LIMIT 10")
                    .fetch_all(&mut *c).await?;
                drop(c);
                for id in ids { if let Err(error) = trigger(&path,&tools,&bus,id,false,ready).await { tracing::warn!(schedule_id=id,%error,"Agent 定时任务未能启动"); } }
                Ok(())
            }.await;
            if let Err(error) = result {
                tracing::warn!(%error,"Agent 定时扫描失败");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ChangedTool;
    #[async_trait::async_trait]
    impl crate::tools::Tool for ChangedTool {
        fn name(&self) -> &'static str {
            "diagnostics.recent"
        }
        fn description(&self) -> &'static str {
            "reloaded tool with undeclared effects"
        }
        async fn call(&self, _: &ToolCtx, _: Value) -> Result<crate::tools::ToolOutput> {
            panic!("unattended unknown-effect tool must never execute")
        }
    }
    fn draft() -> Draft {
        Draft {
            title: "每日自检".into(),
            goal: "检查当前执行状态".into(),
            daily_time: "09:00".into(),
            allowed_tools: vec!["diagnostics.recent".into()],
            max_calls: 4,
            timeout_secs: 60,
        }
    }
    async fn setup() -> (PathBuf, Registry) {
        let dir = std::env::temp_dir().join(format!("yt-agent-schedule-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        let mut c = db::connect(&path).await.unwrap();
        db::migrate(&mut c).await.unwrap();
        let tools = Registry::new();
        tools.register(crate::tools::diagnostics::Diagnostics::new(path.clone()));
        (path, tools)
    }
    #[tokio::test]
    async fn duplicate_ticks_and_manual_runs_share_atomic_task_creation_and_do_not_overlap() {
        let (path, tools) = setup().await;
        let id = register(&path, &tools, "scheduler", draft()).await.unwrap();
        let mut c = db::connect(&path).await.unwrap();
        sqlx::query("UPDATE agent_schedules SET next_run_at=0 WHERE id=?")
            .bind(id)
            .execute(&mut *c)
            .await
            .unwrap();
        drop(c);
        let bus = EventBus::default();
        let (a, b) = tokio::join!(
            trigger(&path, &tools, &bus, id, false, true),
            trigger(&path, &tools, &bus, id, false, true)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(trigger(&path, &tools, &bus, id, true, true).await.is_err());
        assert_eq!(history(&path, id).await.unwrap().len(), 1);
        update(&path, id, Some(false), None, false).await.unwrap();
        assert!(trigger(&path, &tools, &bus, id, true, true).await.is_err());
        // Pausing/deleting the plan must not cancel an already-created task or erase audit.
        update(&path, id, None, None, true).await.unwrap();
        let mut c = db::connect(&path).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks WHERE state='running'")
                .fetch_one(&mut *c)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id IS NULL"
            )
            .fetch_one(&mut *c)
            .await
            .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn permissions_and_model_preflight_leave_dated_failure_without_creating_a_task() {
        let (path, tools) = setup().await;
        let id = register(&path, &tools, "scheduler", draft()).await.unwrap();
        let bus = EventBus::default();
        assert!(trigger(&path, &tools, &bus, id, true, false).await.is_err());
        let mut c = db::connect(&path).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks")
                .fetch_one(&mut *c)
                .await
                .unwrap(),
            0
        );
        assert!(history(&path, id).await.unwrap()[0]["error"]
            .as_str()
            .unwrap()
            .contains("agent_exec"));
        crate::tools::executor::register(&path, tools.get("diagnostics.recent").unwrap().as_ref())
            .await
            .unwrap();
        sqlx::query("UPDATE capability_catalog SET scopes='[\"group:42\"]'")
            .execute(&mut *c)
            .await
            .unwrap();
        assert!(trigger(&path, &tools, &bus, id, true, true).await.is_err());
        assert_eq!(history(&path, id).await.unwrap().len(), 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks")
                .fetch_one(&mut *c)
                .await
                .unwrap(),
            0
        );
        assert!(register(&path, &tools, "scheduler", draft()).await.is_err());
    }
    #[tokio::test]
    async fn failed_task_insert_rolls_back_run_receipt_and_next_execution_time() {
        let (path, tools) = setup().await;
        let id = register(&path, &tools, "scheduler", draft()).await.unwrap();
        let mut c = db::connect(&path).await.unwrap();
        c.execute_batch("UPDATE agent_schedules SET next_run_at=0; CREATE TRIGGER fail_schedule BEFORE INSERT ON task_events BEGIN SELECT RAISE(ABORT,'injected'); END;").await.unwrap();
        assert!(
            trigger(&path, &tools, &EventBus::default(), id, false, true)
                .await
                .is_err()
        );
        assert!(history(&path, id).await.unwrap().is_empty());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks")
                .fetch_one(&mut *c)
                .await
                .unwrap(),
            0
        );
        assert_eq!(list(&path).await.unwrap()[0]["next_run_at"], 0);
    }

    #[tokio::test]
    async fn reloaded_tool_effect_is_checked_again_by_the_actual_executor() {
        let (path, tools) = setup().await;
        let id = register(&path, &tools, "scheduler", draft()).await.unwrap();
        let task = trigger(&path, &tools, &EventBus::default(), id, true, true)
            .await
            .unwrap();
        tools.register(ChangedTool);
        let mut ctx = context("scheduler");
        ctx.task_id = Some(task.clone());
        let checkpoint = agent::runtime::load(&path, &task).await.unwrap().unwrap();
        assert!(
            crate::tools::executor::catalog(&path, &tools, &ctx, &checkpoint.allowed)
                .await
                .unwrap()
                .is_empty()
        );
        let result = crate::tools::executor::execute(
            &path,
            &tools,
            &ctx,
            &checkpoint,
            crate::tools::ToolCall {
                call_id: "test".into(),
                name: "diagnostics.recent".into(),
                arguments: json!({}),
            },
            crate::tools::ToolProtocol::Json,
        )
        .await
        .unwrap();
        assert_eq!(result.result.status, crate::tools::ToolStatus::Rejected);
        assert!(!result.result.external_action_started);
    }
}
