//! First-party scheduled Agent task plugin: durable jobs, at-most-once launch,
//! bounded read-only execution and conservative QQ notification receipts.
//! This module is intentionally in core to inject the existing task bus and SendFn.
use crate::{
    agent::{self, TaskPlan},
    db::{self, SqliteExt},
    event::EventBus,
    reply_engine::{ChatType, SendFn, SendOperation, SendRequest},
};
use anyhow::{bail, ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Mutex, task::JoinHandle};
use yuantuan_plugin_api::{
    Capabilities, Plugin, PluginContext, PluginMeta, PluginPresentation, Tool, ToolCtx, ToolEffect,
    ToolOutput, ToolPolicy, API_VERSION,
};

#[derive(Clone)]
struct Engine {
    db: PathBuf,
    bus: EventBus,
    send: SendFn,
}

struct Scheduler {
    engine: Engine,
    worker: Mutex<Option<JoinHandle<()>>>,
}

pub fn plugin(db: PathBuf, bus: EventBus, send: SendFn) -> Arc<dyn Plugin> {
    Arc::new(Scheduler {
        engine: Engine { db, bus, send },
        worker: Mutex::new(None),
    })
}

#[async_trait]
impl Plugin for Scheduler {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "scheduler",
            name: "知时",
            version: "0.1.0",
            description: "按约定时间创建受限 Agent 任务并私聊投递结果",
            api_version: API_VERSION,
        }
    }

    fn presentation(&self) -> PluginPresentation {
        PluginPresentation {
            purpose: "一次性、每天/每周定时执行只读 Agent 任务",
            poem: "知时而行",
            verse: "有约在先，到时而作。",
            examples: vec![
                "明天早上九点提醒我核查项目进度",
                "每天北京时间二十一点总结一次 AI 新闻",
                "查看我的定时任务",
            ],
        }
    }

    async fn start(&self, _ctx: PluginContext, _config: Value) -> Result<Capabilities> {
        ensure_schema(&self.engine.db).await?;
        // A committed claim may or may not have started an external action.
        // Never replay a claim after restart.
        let mut c = db::connect(&self.engine.db).await?;
        sqlx::query("UPDATE scheduled_runs SET state='uncertain', result_text='重启前已领取任务，但不能确认是否启动；不会自动重放', delivery='ready' WHERE state='claimed'")
            .execute(&mut *c).await?;
        sqlx::query("UPDATE scheduled_runs SET delivery='unknown' WHERE delivery='sending'")
            .execute(&mut *c).await?;
        drop(c);

        let mut worker = self.worker.lock().await;
        if let Some(previous) = worker.take() {
            previous.abort();
            let _ = previous.await;
        }
        let engine = self.engine.clone();
        *worker = Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if let Err(error) = engine.cycle().await {
                    tracing::warn!(%error, "scheduler cycle failed; retained in SQLite");
                }
            }
        }));
        let tools = [
            Action::Create,
            Action::List,
            Action::Pause,
            Action::Resume,
            Action::Cancel,
            Action::History,
        ]
        .into_iter()
        .map(|action| {
            Arc::new(ScheduleTool {
                db: self.engine.db.clone(),
                action,
            }) as Arc<dyn Tool>
        })
        .collect();
        Ok(Capabilities {
            tools,
            skills: vec![],
        })
    }

    async fn stop(&self) -> Result<()> {
        if let Some(worker) = self.worker.lock().await.take() {
            worker.abort();
            let _ = worker.await;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Action {
    Create,
    List,
    Pause,
    Resume,
    Cancel,
    History,
}

struct ScheduleTool {
    db: PathBuf,
    action: Action,
}

#[async_trait]
impl Tool for ScheduleTool {
    fn name(&self) -> &'static str {
        match self.action {
            Action::Create => "scheduler.create",
            Action::List => "scheduler.list",
            Action::Pause => "scheduler.pause",
            Action::Resume => "scheduler.resume",
            Action::Cancel => "scheduler.cancel",
            Action::History => "scheduler.history",
        }
    }

    fn description(&self) -> &'static str {
        match self.action {
            Action::Create => "按 RFC3339 日期时间（包含 Z 或明确 UTC 偏移）创建未来一次、每天或每周重复的只读任务；默认私聊通知创建者",
            Action::List => "列出当前用户拥有的定时任务和下次运行时间",
            Action::Pause => "暂停自己的定时任务；不会取消已经开始执行的 Agent",
            Action::Resume => "恢复自己的定时任务，过期的单次任务移至一分钟后",
            Action::Cancel => "取消自己的定时任务，阻止后续触发",
            Action::History => "查询自己定时任务的真实运行、结果和投递状态",
        }
    }

    fn policy(&self) -> ToolPolicy {
        ToolPolicy {
            effect: match self.action {
                Action::List | Action::History => ToolEffect::ReadOnly,
                _ => ToolEffect::WorkspaceWrite,
            },
            ..Default::default()
        }
    }

    fn parameters_schema(&self) -> Value {
        match self.action {
            Action::Create => json!({
                "type": "object",
                "properties": {
                    "goal": {"type":"string","minLength":1,"maxLength":3000},
                    "when": {"type":"string","description":"RFC3339, e.g. 2026-11-17T09:00:00+08:00; MUST include UTC offset or Z"},
                    "repeat": {"type":"string","enum":["none","daily_fixed","weekly_fixed"],"description":"Fixed 24h/7d UTC duration; does not adjust for daylight saving time"},
                    "use_search": {"type":"boolean","description":"Allow only anysearch.search and anysearch.extract for the scheduled Agent"}
                },
                "required": ["goal","when"],
                "additionalProperties": false
            }),
            Action::List => json!({"type":"object","additionalProperties":false}),
            _ => json!({
                "type":"object",
                "properties":{"job_id":{"type":"string","minLength":1}},
                "required":["job_id"],
                "additionalProperties":false
            }),
        }
    }

    fn available_in_chat(&self, ctx: &ToolCtx) -> bool {
        valid_caller(ctx)
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        ensure!(valid_caller(ctx), "仅允许已识别 QQ 用户创建或查询定时任务");
        let data = match self.action {
            Action::Create => self.create(ctx, &args).await?,
            Action::List => self.list(ctx).await?,
            Action::Pause => self.change(ctx, &args, "paused").await?,
            Action::Resume => self.change(ctx, &args, "active").await?,
            Action::Cancel => self.change(ctx, &args, "cancelled").await?,
            Action::History => self.history(ctx, &args).await?,
        };
        Ok(ToolOutput {
            summary: match self.action {
                Action::Create => "定时任务已持久保存；执行时间与私聊投递目标见数据",
                Action::List => "已读取当前用户的定时任务",
                Action::History => "已读取定时执行及投递回执",
                _ => "定时任务状态已更新",
            }
            .into(),
            artifacts: vec![],
            data,
        })
    }
}

fn valid_caller(ctx: &ToolCtx) -> bool {
    matches!(ctx.chat_type.as_str(), "private" | "group")
        && ctx
            .sender_pid
            .strip_prefix("p_")
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|id| id > 0)
        && !ctx.chat_id.is_empty()
}

impl ScheduleTool {
    async fn create(&self, ctx: &ToolCtx, v: &Value) -> Result<Value> {
        let goal = v["goal"].as_str().unwrap_or("").trim();
        ensure!(
            !goal.is_empty() && goal.chars().count() <= 3000,
            "任务目标必须为 1–3000 字"
        );
        let when = v["when"].as_str().unwrap_or("");
        let run_at = parse_rfc3339(when)?;
        let now = now_secs();
        ensure!(run_at >= now + 30, "执行时间必须至少在 30 秒之后");
        ensure!(run_at <= now + 366 * 86_400, "首次执行时间最多可提前一年设置");
        let repeat = v["repeat"].as_str().unwrap_or("none");
        let interval = match repeat {
            "none" => 0_i64,
            "daily_fixed" => 86_400,
            "weekly_fixed" => 7 * 86_400,
            _ => bail!("重复频率只支持 none / daily_fixed / weekly_fixed"),
        };
        let use_search = v["use_search"].as_bool().unwrap_or(false);
        let target: u64 = ctx.sender_pid[2..].parse()?;
        let id = format!("sj_{:x}_{:016x}", now, rand::random::<u64>());
        let mut c = db::connect(&self.db).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scheduled_jobs WHERE owner_pid=? AND state IN ('active','paused')",
        )
        .bind(&ctx.sender_pid)
        .fetch_one(&mut *c)
        .await?;
        ensure!(count < 32, "每位用户最多保留 32 个活跃或暂停的定时任务");
        sqlx::query(
            "INSERT INTO scheduled_jobs(job_id,owner_pid,chat_id,chat_type,target_qq,goal,next_run_at,when_label,interval_secs,use_search,state,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,'active',?)",
        )
        .bind(&id)
        .bind(&ctx.sender_pid)
        .bind(&ctx.chat_id)
        .bind(&ctx.chat_type)
        .bind(target as i64)
        .bind(goal)
        .bind(run_at)
        .bind(when)
        .bind(interval)
        .bind(if use_search { 1 } else { 0 })
        .bind(now)
        .execute(&mut *c)
        .await?;
        Ok(json!({
            "job_id":id,"goal":goal,"when":when,"next_run_utc_seconds":run_at,
            "repeat":repeat,"use_search":use_search,"state":"active",
            "delivery":"private_to_creator",
            "execution_permissions": if use_search { "only anysearch.search and anysearch.extract" } else { "no tools (LLM only)" },
            "notice":"Daily/weekly repeats are fixed UTC intervals, not DST-adjusted local wall clock times."
        }))
    }

    async fn list(&self, ctx: &ToolCtx) -> Result<Value> {
        let mut c = db::connect(&self.db).await?;
        let rows = sqlx::query(
            "SELECT job_id,goal,when_label,next_run_at,interval_secs,use_search,state FROM scheduled_jobs WHERE owner_pid=? AND (?='private' OR chat_id=?) ORDER BY created_at DESC LIMIT 64",
        )
        .bind(&ctx.sender_pid)
        .bind(&ctx.chat_type)
        .bind(&ctx.chat_id)
        .fetch_all(&mut *c)
        .await?;
        Ok(json!({"jobs": rows.iter().map(|r| json!({
            "job_id":r.get::<String,_>("job_id"),
            "goal":r.get::<String,_>("goal"),
            "original_time":r.get::<String,_>("when_label"),
            "next_run_utc_seconds":r.get::<i64,_>("next_run_at"),
            "interval_seconds":r.get::<i64,_>("interval_secs"),
            "use_search":r.get::<i64,_>("use_search")!=0,
            "state":r.get::<String,_>("state"),
        })).collect::<Vec<_>>()}))
    }

    async fn change(&self, ctx: &ToolCtx, v: &Value, state: &str) -> Result<Value> {
        let id = v["job_id"].as_str().unwrap_or("");
        ensure!(id.starts_with("sj_") && id.len() < 80, "job_id 无效");
        let mut c = db::connect(&self.db).await?;
        let changed = if state == "active" {
            sqlx::query(
                "UPDATE scheduled_jobs SET state='active', next_run_at=CASE WHEN next_run_at<=? THEN ? ELSE next_run_at END WHERE job_id=? AND owner_pid=? AND (?='private' OR chat_id=?) AND state='paused'",
            )
            .bind(now_secs())
            .bind(now_secs() + 60)
            .bind(id)
            .bind(&ctx.sender_pid)
            .bind(&ctx.chat_type)
            .bind(&ctx.chat_id)
            .execute(&mut *c)
            .await?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE scheduled_jobs SET state=? WHERE job_id=? AND owner_pid=? AND (?='private' OR chat_id=?) AND state IN ('active','paused')",
            )
            .bind(state)
            .bind(id)
            .bind(&ctx.sender_pid)
            .bind(&ctx.chat_type)
            .bind(&ctx.chat_id)
            .execute(&mut *c)
            .await?
            .rows_affected()
        };
        ensure!(changed == 1, "任务不存在、无权限或状态不允许此操作");
        Ok(json!({"job_id":id,"state":state,"notice":"Already launched external actions are not undone."}))
    }

    async fn history(&self, ctx: &ToolCtx, v: &Value) -> Result<Value> {
        let id = v["job_id"].as_str().unwrap_or("");
        let mut c = db::connect(&self.db).await?;
        let rows = sqlx::query(
            "SELECT r.scheduled_for,r.task_id,r.state,r.delivery,r.result_text FROM scheduled_runs r JOIN scheduled_jobs j ON j.job_id=r.job_id WHERE j.job_id=? AND j.owner_pid=? AND (?='private' OR j.chat_id=?) ORDER BY r.scheduled_for DESC LIMIT 20",
        )
        .bind(id)
        .bind(&ctx.sender_pid)
        .bind(&ctx.chat_type)
        .bind(&ctx.chat_id)
        .fetch_all(&mut *c)
        .await?;
        Ok(json!({"job_id":id,"runs":rows.iter().map(|r| json!({
            "scheduled_for_utc_seconds":r.get::<i64,_>("scheduled_for"),
            "task_id":r.get::<Option<String>,_>("task_id"),
            "state":r.get::<String,_>("state"),
            "delivery":r.get::<String,_>("delivery"),
            "summary":r.get::<Option<String>,_>("result_text")
        })).collect::<Vec<_>>()}))
    }
}

async fn ensure_schema(path: &Path) -> Result<()> {
    let mut c = db::connect(path).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS scheduled_jobs (
            job_id TEXT PRIMARY KEY, owner_pid TEXT NOT NULL, chat_id TEXT NOT NULL,
            chat_type TEXT NOT NULL, target_qq INTEGER NOT NULL, goal TEXT NOT NULL,
            next_run_at INTEGER NOT NULL, when_label TEXT NOT NULL,
            interval_secs INTEGER NOT NULL DEFAULT 0, use_search INTEGER NOT NULL DEFAULT 0,
            state TEXT NOT NULL, created_at INTEGER NOT NULL
        )",
    )
    .execute(&mut *c)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS scheduled_jobs_due ON scheduled_jobs(state,next_run_at)")
        .execute(&mut *c)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS scheduled_runs (
            job_id TEXT NOT NULL REFERENCES scheduled_jobs(job_id),
            scheduled_for INTEGER NOT NULL, task_id TEXT, state TEXT NOT NULL,
            delivery TEXT NOT NULL DEFAULT 'none', result_text TEXT,
            created_at INTEGER NOT NULL, PRIMARY KEY(job_id, scheduled_for)
        )",
    )
    .execute(&mut *c)
    .await?;
    Ok(())
}

impl Engine {
    async fn cycle(&self) -> Result<()> {
        self.launch_due().await?;
        self.collect_finished().await?;
        self.deliver().await?;
        Ok(())
    }

    async fn launch_due(&self) -> Result<()> {
        let now = now_secs();
        let mut c = db::connect(&self.db).await?;
        let ids: Vec<String> = sqlx::query(
            "SELECT job_id FROM scheduled_jobs WHERE state='active' AND next_run_at<=? ORDER BY next_run_at LIMIT 4",
        )
        .bind(now)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(|r| r.get::<String, _>("job_id"))
        .collect();
        drop(c);
        for id in ids {
            if let Err(error) = self.launch_one(&id, now).await {
                tracing::warn!(job_id=%id,%error,"scheduled run launch failed");
            }
        }
        Ok(())
    }

    async fn launch_one(&self, id: &str, now: i64) -> Result<()> {
        let mut c = db::connect(&self.db).await?;
        let mut tx = c.begin_immediate().await?;
        let Some(row) = sqlx::query(
            "SELECT owner_pid,chat_id,chat_type,goal,next_run_at,interval_secs,use_search FROM scheduled_jobs WHERE job_id=? AND state='active'",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(());
        };
        let due: i64 = row.get("next_run_at");
        if due > now {
            return Ok(());
        }
        let interval: i64 = row.get("interval_secs");
        let owner: String = row.get("owner_pid");
        let chat: String = row.get("chat_id");
        let chat_type: String = row.get("chat_type");
        let goal: String = row.get("goal");
        let use_search: bool = row.get::<i64, _>("use_search") != 0;
        let next = if interval == 0 {
            0
        } else {
            due + ((now - due) / interval + 1) * interval
        };
        let next_state = if interval == 0 { "completed" } else { "active" };
        let changed = sqlx::query(
            "UPDATE scheduled_jobs SET next_run_at=?,state=? WHERE job_id=? AND state='active' AND next_run_at=?",
        )
        .bind(next)
        .bind(next_state)
        .bind(id)
        .bind(due)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed != 1 {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO scheduled_runs(job_id,scheduled_for,state,delivery,created_at) VALUES(?,?,'claimed','none',?)",
        )
        .bind(id)
        .bind(due)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        // The durable claim was committed first. If killed after this line and before
        // recording task_id, startup classifies it as uncertain rather than replaying.
        let plan = TaskPlan {
            allowed_tools: if use_search {
                vec!["anysearch.search".into(), "anysearch.extract".into()]
            } else {
                vec!["scheduler.no_tools".into()]
            },
            ..TaskPlan::default()
        };
        let result = agent::create_scoped_task(
            &self.db,
            &self.bus,
            &chat,
            &chat_type,
            &owner,
            None, // scheduled actions have no live QQ message anchor
            &goal,
            plan,
            json!({"source":"scheduler","job_id":id,"scheduled_for_utc":due}),
        )
        .await;
        let mut c = db::connect(&self.db).await?;
        match result {
            Ok(task_id) => {
                sqlx::query(
                    "UPDATE scheduled_runs SET state='running',task_id=? WHERE job_id=? AND scheduled_for=? AND state='claimed'",
                )
                .bind(&task_id)
                .bind(id)
                .bind(due)
                .execute(&mut *c)
                .await?;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE scheduled_runs SET state='failed',delivery='ready',result_text=? WHERE job_id=? AND scheduled_for=? AND state='claimed'",
                )
                .bind(format!("无法创建 Agent 任务：{error}").chars().take(800).collect::<String>())
                .bind(id)
                .bind(due)
                .execute(&mut *c)
                .await?;
            }
        }
        Ok(())
    }

    async fn collect_finished(&self) -> Result<()> {
        let mut c = db::connect(&self.db).await?;
        let rows = sqlx::query(
            "SELECT r.job_id,r.scheduled_for,t.state AS task_state, tr.phase,tr.final_text,tr.stop_reason
             FROM scheduled_runs r JOIN tasks t ON t.task_id=r.task_id
             JOIN task_runtime tr ON tr.task_id=t.task_id
             WHERE r.state='running' AND (t.state IN ('finished','failed') OR tr.phase='waiting_input') LIMIT 10",
        )
        .fetch_all(&mut *c)
        .await?;
        for row in rows {
            let state: String = row.get("task_state");
            let phase: String = row.get("phase");
            let state = if phase == "waiting_input" {
                "needs_input"
            } else if state == "finished" {
                "succeeded"
            } else {
                "failed"
            };
            let text: Option<String> = if state == "failed" {
                row.try_get("stop_reason")?
            } else {
                row.try_get("final_text")?
            };
            sqlx::query(
                "UPDATE scheduled_runs SET state=?,result_text=?,delivery='ready'
                 WHERE job_id=? AND scheduled_for=? AND state='running'",
            )
            .bind(state)
            .bind(text.unwrap_or_else(|| state.to_owned()).chars().take(1800).collect::<String>())
            .bind(row.get::<String, _>("job_id"))
            .bind(row.get::<i64, _>("scheduled_for"))
            .execute(&mut *c)
            .await?;
        }
        Ok(())
    }

    async fn deliver(&self) -> Result<()> {
        let mut c = db::connect(&self.db).await?;
        let rows = sqlx::query(
            "SELECT r.job_id,r.scheduled_for,r.task_id,r.state,r.result_text,j.goal,j.target_qq
             FROM scheduled_runs r JOIN scheduled_jobs j ON j.job_id=r.job_id
             WHERE r.delivery='ready' ORDER BY r.created_at LIMIT 5",
        )
        .fetch_all(&mut *c)
        .await?;
        for row in rows {
            let id: String = row.get("job_id");
            let due: i64 = row.get("scheduled_for");
            let state: String = row.get("state");
            let content: Option<String> = row.try_get("result_text")?;
            let goal: String = row.get("goal");
            let target: i64 = row.get("target_qq");
            let claimed = sqlx::query(
                "UPDATE scheduled_runs SET delivery='sending' WHERE job_id=? AND scheduled_for=? AND delivery='ready'",
            )
            .bind(&id)
            .bind(due)
            .execute(&mut *c)
            .await?
            .rows_affected();
            if claimed != 1 {
                continue;
            }
            let text = format!(
                "[云团定时任务]\n任务：{}\n计划时间：UTC {}（Unix 秒）\n执行状态：{}\n结果：{}\n任务编号：{}",
                goal.chars().take(250).collect::<String>(),
                due,
                state,
                content.unwrap_or_else(|| "无结果".into()).chars().take(1500).collect::<String>(),
                id,
            );
            let sent = (self.send)(SendRequest {
                chat_type: ChatType::Private,
                target: target as u64,
                segments: json!([{"type":"text","data":{"text":text}}]),
                operation: SendOperation::Message,
            })
            .await;
            let confirmed = sent
                .as_ref()
                .ok()
                .and_then(|v| v["message_id"].as_i64())
                .is_some_and(|value| value > 0);
            sqlx::query(
                "UPDATE scheduled_runs SET delivery=? WHERE job_id=? AND scheduled_for=? AND delivery='sending'",
            )
            .bind(if confirmed { "sent" } else { "unknown" })
            .bind(&id)
            .bind(due)
            .execute(&mut *c)
            .await?;
        }
        Ok(())
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a restricted RFC3339 second-precision timestamp with an explicit zone.
/// No dependency on the OS timezone or the model's interpretation of local time.
fn parse_rfc3339(s: &str) -> Result<i64> {
    let b = s.as_bytes();
    ensure!(b.len() == 20 || b.len() == 25, "时间必须是带时区的 RFC3339 秒精度格式");
    ensure!(
        b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':',
        "无效的日期时间格式"
    );
    let number = |start: usize, end: usize| -> Result<i64> {
        let chunk = b.get(start..end).ok_or_else(|| anyhow::anyhow!("日期字段缺失"))?;
        ensure!(chunk.iter().all(u8::is_ascii_digit), "时间字段必须是数字");
        Ok(std::str::from_utf8(chunk)?.parse()?)
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    ensure!((2020..=2100).contains(&year) && (1..=12).contains(&month), "日期超出支持范围");
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let lengths = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    ensure!(day >= 1 && day <= lengths[(month - 1) as usize], "日期无效");
    ensure!(hour <= 23 && minute <= 59 && second <= 59, "时间无效");
    let offset_minutes = if b.len() == 20 {
        ensure!(b[19] == b'Z', "需要明确的 UTC 时区");
        0
    } else {
        ensure!(
            (b[19] == b'+' || b[19] == b'-') && b[22] == b':',
            "UTC 偏移必须为 +HH:MM 或 -HH:MM"
        );
        let zh = number(20, 22)?;
        let zm = number(23, 25)?;
        ensure!(zh <= 14 && zm <= 59 && !(zh == 14 && zm != 0), "UTC 偏移超出范围");
        (zh * 60 + zm) * if b[19] == b'+' { 1 } else { -1 }
    };
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let unix_days = era * 146_097 + doe - 719_468;
    Ok(unix_days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezones_are_explicit_and_equal() {
        assert_eq!(
            parse_rfc3339("2026-11-16T10:00:00+08:00").unwrap(),
            parse_rfc3339("2026-11-16T02:00:00Z").unwrap()
        );
    }
    #[test]
    fn invalid_dates_are_rejected() {
        assert!(parse_rfc3339("2026-02-29T10:00:00Z").is_err());
        assert!(parse_rfc3339("2026-11-16T10:00:00").is_err());
        assert!(parse_rfc3339("2026-11-16T10:00:00+18:00").is_err());
    }
    #[test]
    fn only_qq_callers_are_accepted() {
        assert!(!valid_caller(&ToolCtx {
            chat_id:"123".into(),chat_type:"group".into(),
            sender_pid:"self".into(),task_id:None,locale:None
        }));
        assert!(valid_caller(&ToolCtx {
            chat_id:"123".into(),chat_type:"group".into(),
            sender_pid:"p_456".into(),task_id:None,locale:None
        }));
    }
}
