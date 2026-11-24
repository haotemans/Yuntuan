//! Persisted daily scheduling of read-only tools. No model required for recurring jobs.
//! Each invocation goes through the already-registered HostedTool and capability policy.
use crate::{
    capabilities, db,
    tools::{Registry, Tool, ToolCtx, ToolEffect},
};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const QUARK_INSPECT: &str = "quark-drive.inspect_share";
const MAX_DUE_PER_TICK: i64 = 10;
const LEASE_SECS: i64 = 300;
const BEIJING_OFFSET: i64 = 480;

#[derive(Clone, Copy, Serialize)]
struct TaskKind {
    tool_name: &'static str,
    label: &'static str,
    input: Option<&'static str>,
}

const TASK_KINDS: &[TaskKind] = &[
    TaskKind { tool_name: QUARK_INSPECT, label: "检查夸克分享链接", input: Some("url") },
    TaskKind { tool_name: "quark-drive.search", label: "搜索夸克网盘", input: Some("keyword") },
    TaskKind { tool_name: "quark-drive.share_updates", label: "查看夸克分享更新", input: None },
    TaskKind { tool_name: "quark-drive.updated_files", label: "查看指定夸克分享的更新文件", input: Some("url") },
    TaskKind { tool_name: "tencent-docs.catalog", label: "检查腾讯文档连接", input: None },
];

#[derive(Serialize)]
pub struct TaskOption {
    #[serde(flatten)]
    kind: TaskKind,
    pub unavailable: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduledItem {
    pub id: i64,
    pub title: String,
    pub tool_name: String,
    pub resource_key: String,
    pub enabled: bool,
    pub local_hour: i64,
    pub local_minute: i64,
    pub timezone_offset_minutes: i64,
    pub running: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub last_status: String,
    pub last_summary: String,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "scheduled".into(),
        chat_type: "private".into(),
        sender_pid: "scheduler".into(),
        locale: Some("zh-CN".into()),
    }
}
pub fn next_daily(now: i64, offset_minutes: i64, hour: i64, minute: i64) -> i64 {
    let offset = offset_minutes * 60;
    let day = (now + offset).div_euclid(86_400) * 86_400;
    let target = day + hour * 3600 + minute * 60 - offset;
    if target > now {
        target
    } else {
        target + 86_400
    }
}
pub fn validate_quark_url(url: &str) -> Result<String> {
    let url = url.trim();
    ensure!(
        url.len() <= 1024 && !url.contains(char::is_whitespace),
        "链接格式无效"
    );
    let parsed = reqwest::Url::parse(url).context("夸克分享 URL 无效")?;
    ensure!(
        parsed.scheme() == "https"
            && parsed.host_str() == Some("pan.quark.cn")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path().starts_with("/s/")
            && parsed.path().len() > 3
            && parsed.fragment().is_none(),
        "仅接受夸克网盘官方 HTTPS 分享链接"
    );
    Ok(url.to_owned())
}
pub async fn list_quark(path: &Path) -> Result<Vec<ScheduledItem>> {
    list_tasks(path, Some(QUARK_INSPECT)).await
}

pub async fn list_tasks(path: &Path, tool: Option<&str>) -> Result<Vec<ScheduledItem>> {
    let mut c = db::connect(path).await?;
    let rows=sqlx::query("SELECT id,title,tool_name,resource_key,enabled,next_run_at,last_run_at,last_status,last_summary,
        local_hour,local_minute,timezone_offset_minutes,COALESCE(last_status='checking' AND claimed_at>unixepoch()-?2,0) AS running
        FROM scheduled_tools WHERE (?1 IS NULL OR tool_name=?1) ORDER BY id DESC LIMIT 600")
        .bind(tool).bind(LEASE_SECS).fetch_all(&mut *c).await?;
    rows.into_iter()
        .map(|row| {
            Ok(ScheduledItem {
                id: row.try_get("id")?,
                title: row.try_get("title")?,
                tool_name: row.try_get("tool_name")?,
                resource_key: row.try_get("resource_key")?,
                enabled: row.try_get::<i64, _>("enabled")? != 0,
                local_hour: row.try_get("local_hour")?,
                local_minute: row.try_get("local_minute")?,
                timezone_offset_minutes: row.try_get("timezone_offset_minutes")?,
                running: row.try_get("running")?,
                next_run_at: row.try_get("next_run_at")?,
                last_run_at: row.try_get("last_run_at")?,
                last_status: row.try_get("last_status")?,
                last_summary: row.try_get("last_summary")?,
            })
        })
        .collect()
}

pub async fn task_options(path: &Path, tools: &Registry) -> Result<Vec<TaskOption>> {
    let context = ctx();
    let mut options = Vec::new();
    for &kind in TASK_KINDS {
        let unavailable = match tools.get(kind.tool_name) {
            None => Some("请先启用并配置插件"),
            Some(tool) if tool.policy().effect != ToolEffect::ReadOnly => Some("该工具未声明为只读"),
            Some(tool) if !tool.available_in_chat(&context) => Some("插件范围不允许调度器"),
            Some(_) if !capabilities::allowed(path, kind.tool_name, "tool", &context).await? => {
                Some("工具未启用或未授权 private:scheduled")
            }
            Some(_) => None,
        };
        options.push(TaskOption { kind, unavailable });
    }
    Ok(options)
}
async fn ensure_readonly(path: &Path, tools: &Registry, name: &str, args: &Value) -> Result<Arc<dyn Tool>> {
    let tool = tools
        .get(name)
        .context("对应插件未启用或尚未完成授权配置")?;
    ensure!(
        tool.policy().effect == ToolEffect::ReadOnly,
        "定时任务首期只允许只读 Tool"
    );
    ensure!(tool.available_in_chat(&ctx()), "当前工具不对调度器开放");
    capabilities::check_tool(path, name, &ctx(), &tool.parameters_schema(), args).await?;
    Ok(tool)
}

pub async fn register_quark(path: &Path, tools: &Registry, url: &str, title: &str) -> Result<i64> {
    let url = validate_quark_url(url)?;
    let label = title.trim();
    ensure!(label.chars().count() <= 100, "任务标题过长");
    let args = json!({"url":url});
    ensure_readonly(path, tools, QUARK_INSPECT, &args).await?;
    let mut c = db::connect(path).await?;
    let due = next_daily(now(), BEIJING_OFFSET, 9, 0);
    let row=sqlx::query("INSERT INTO scheduled_tools(title,tool_name,resource_key,arguments_json,timezone_offset_minutes,local_hour,local_minute,next_run_at) SELECT ?,?,?,?,480,9,0,? WHERE (SELECT count(*) FROM scheduled_tools WHERE tool_name=?)<200 OR EXISTS(SELECT 1 FROM scheduled_tools WHERE tool_name=? AND resource_key=?) ON CONFLICT(tool_name,resource_key) DO UPDATE SET title=excluded.title,enabled=1 RETURNING id")
        .bind(if label.is_empty(){"夸克分享链接"}else{label})
        .bind(QUARK_INSPECT).bind(&url).bind(args.to_string()).bind(due)
        .bind(QUARK_INSPECT).bind(QUARK_INSPECT).bind(&url)
        .fetch_optional(&mut *c).await?.context("最多登记200条分享检查任务，请先删除不用的记录")?;
    Ok(row.try_get("id")?)
}

fn daily_time(value: &str) -> Result<(i64, i64)> {
    let (hours, minutes) = value.split_once(':').context("请输入 HH:MM 格式的执行时间")?;
    ensure!(hours.len() == 2 && minutes.len() == 2 && value.bytes().all(|b| b.is_ascii_digit() || b == b':'), "请输入 HH:MM 格式的执行时间");
    let (hour, minute) = (hours.parse()?, minutes.parse()?);
    ensure!((0..24).contains(&hour) && (0..60).contains(&minute), "每日执行时间无效");
    Ok((hour, minute))
}

pub async fn register_task(path: &Path, tools: &Registry, name: &str, input: &str, title: &str, time: &str) -> Result<i64> {
    let kind = TASK_KINDS.iter().find(|kind| kind.tool_name == name).context("该工具未开放定时执行")?;
    let (hour, minute) = daily_time(time)?;
    let title = title.trim();
    ensure!(title.chars().count() <= 100, "任务标题过长");
    let input = input.trim();
    let (key, arguments) = match kind.input {
        Some("url") => {
            let url = validate_quark_url(input)?;
            (url.clone(), json!({"url":url}))
        }
        Some("keyword") => {
            ensure!((1..=50).contains(&input.chars().count()), "请输入 1 到 50 个字符的关键词");
            (input.to_owned(), json!({"keyword":input}))
        }
        _ => {
            ensure!(input.is_empty(), "此工具无需参数");
            (String::new(), json!({}))
        }
    };
    ensure_readonly(path, tools, name, &arguments).await?;
    let due = next_daily(now(), BEIJING_OFFSET, hour, minute);
    let mut conn = db::connect(path).await?;
    let inserted: Option<i64> = sqlx::query_scalar("INSERT INTO scheduled_tools(title,tool_name,resource_key,arguments_json,timezone_offset_minutes,local_hour,local_minute,next_run_at)
        SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE (SELECT COUNT(*) FROM scheduled_tools)<400
        AND (?2!='quark-drive.inspect_share' OR (SELECT COUNT(*) FROM scheduled_tools WHERE tool_name=?2)<200)
        ON CONFLICT(tool_name,resource_key) DO NOTHING RETURNING id")
        .bind(if title.is_empty() { kind.label } else { title }).bind(name).bind(&key).bind(arguments.to_string())
        .bind(BEIJING_OFFSET).bind(hour).bind(minute).bind(due).fetch_optional(&mut *conn).await?;
    if let Some(id) = inserted { return Ok(id); }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM scheduled_tools WHERE tool_name=? AND resource_key=?)")
        .bind(name).bind(key).fetch_one(&mut *conn).await?;
    if exists { bail!("相同工具和参数已有任务，可直接调整执行时间"); }
    bail!("定时任务数量已达上限")
}

pub async fn set_task_time(path: &Path, id: i64, time: &str) -> Result<bool> {
    let (hour, minute) = daily_time(time)?;
    ensure!(id > 0, "任务编号无效");
    let at = now();
    let due = next_daily(at, BEIJING_OFFSET, hour, minute);
    let mut conn = db::connect(path).await?;
    Ok(sqlx::query("UPDATE scheduled_tools SET local_hour=?1,local_minute=?2,timezone_offset_minutes=?3,next_run_at=?4,
        last_status=CASE WHEN last_status='checking' THEN 'pending' ELSE last_status END,claimed_at=NULL
        WHERE id=?5 AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?6)")
        .bind(hour).bind(minute).bind(BEIJING_OFFSET).bind(due).bind(id).bind(at-LEASE_SECS)
        .execute(&mut *conn).await?.rows_affected() == 1)
}
pub async fn toggle(path: &Path, id: i64, enabled: bool) -> Result<bool> {
    toggle_task(path, id, enabled, Some(QUARK_INSPECT)).await
}

pub async fn toggle_task(path: &Path, id: i64, enabled: bool, tool: Option<&str>) -> Result<bool> {
    ensure!(id > 0, "任务编号无效");
    let mut c = db::connect(path).await?;
    let n = sqlx::query("UPDATE scheduled_tools SET enabled=?1 WHERE id=?2 AND (?3 IS NULL OR tool_name=?3)")
        .bind(enabled)
        .bind(id)
        .bind(tool)
        .execute(&mut *c)
        .await?
        .rows_affected();
    Ok(n == 1)
}
pub async fn remove(path: &Path, id: i64) -> Result<bool> {
    remove_task(path, id, Some(QUARK_INSPECT)).await
}

pub async fn remove_task(path: &Path, id: i64, tool: Option<&str>) -> Result<bool> {
    ensure!(id > 0, "任务编号无效");
    let mut c = db::connect(path).await?;
    let n = sqlx::query("DELETE FROM scheduled_tools WHERE id=?1 AND (?2 IS NULL OR tool_name=?2)
        AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?3)")
        .bind(id)
        .bind(tool)
        .bind(now()-LEASE_SECS)
        .execute(&mut *c)
        .await?
        .rows_affected();
    Ok(n == 1)
}

async fn execute_one(path: &Path, tools: &Registry, id: i64, force: bool, only_tool: Option<&str>) -> Result<()> {
    let started = now();
    let mut c = db::connect(path).await?;
    let claimed=sqlx::query("UPDATE scheduled_tools SET last_status='checking',claimed_at=? WHERE id=? AND enabled=1 AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?) AND (?=1 OR next_run_at<=?) AND (?6 IS NULL OR tool_name=?6)")
        .bind(started).bind(id).bind(started-LEASE_SECS)
        .bind(i64::from(force)).bind(started).bind(only_tool).execute(&mut *c).await?.rows_affected();
    if claimed == 0 {
        bail!("任务未到执行时间、已停用或正在运行");
    }
    let row=sqlx::query("SELECT tool_name,arguments_json,timezone_offset_minutes,local_hour,local_minute FROM scheduled_tools WHERE id=?")
        .bind(id).fetch_one(&mut *c).await?;
    let tool_name: String = row.try_get("tool_name")?;
    let args: String = row.try_get("arguments_json")?;
    let offset: i64 = row.try_get("timezone_offset_minutes")?;
    let hour: i64 = row.try_get("local_hour")?;
    let minute: i64 = row.try_get("local_minute")?;
    drop(c);
    let work = async {
        let args: Value = serde_json::from_str(&args).context("已登记的定时任务参数损坏")?;
        let tool = ensure_readonly(path, tools, &tool_name, &args).await?;
        let out = tokio::time::timeout(Duration::from_secs(120), tool.call(&ctx(), args))
            .await
            .context("执行超时")??;
        // Don't persist private file listings; only an actual success receipt.
        Ok::<String, anyhow::Error>(if crate::consolidation::is_sensitive(&out.summary) {
            "执行完成，敏感结果已隐藏".into()
        } else {
            out.summary.chars().take(400).collect()
        })
    }
    .await;
    let (status, summary) = match work {
        Ok(message) => ("accessible", message),
        Err(error) => {
            let detail = error.to_string();
            let message = if crate::consolidation::is_sensitive(&detail) {
                "检查失败，请核对插件连接和权限".into()
            } else {
                format!("检查失败：{}", detail.chars().take(200).collect::<String>())
            };
            ("check_failed", message)
        }
    };
    let finished = now();
    let next = next_daily(finished, offset, hour, minute);
    let mut c = db::connect(path).await?;
    let n=sqlx::query("UPDATE scheduled_tools SET last_run_at=?,last_status=?,last_summary=?,next_run_at=?,claimed_at=NULL WHERE id=? AND claimed_at=?")
        .bind(finished).bind(status).bind(summary).bind(next).bind(id).bind(started)
        .execute(&mut *c).await?.rows_affected();
    ensure!(n == 1, "定时任务完成时状态已改变，结果未覆盖新任务");
    Ok(())
}
pub async fn check_now(path: &Path, tools: &Registry, id: i64) -> Result<()> {
    run_now(path, tools, id, Some(QUARK_INSPECT)).await
}

pub async fn run_now(path: &Path, tools: &Registry, id: i64, tool: Option<&str>) -> Result<()> {
    ensure!(id > 0, "任务编号无效");
    execute_one(path, tools, id, true, tool).await
}
pub fn spawn(path: PathBuf, tools: Registry) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = run_due(&path, &tools).await {
                tracing::warn!(error=%error,"每日只读工具调度失败");
            }
        }
    })
}
async fn run_due(path: &Path, tools: &Registry) -> Result<()> {
    let time = now();
    let mut c = db::connect(path).await?;
    let ids:Vec<i64>=sqlx::query_scalar("SELECT id FROM scheduled_tools WHERE enabled=1 AND next_run_at<=? AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?) ORDER BY next_run_at,id LIMIT ?")
        .bind(time).bind(time-LEASE_SECS).bind(MAX_DUE_PER_TICK).fetch_all(&mut *c).await?;
    drop(c);
    for id in ids {
        if let Err(error) = execute_one(path, tools, id, false, None).await {
            tracing::warn!(job=id,error=%error,"已登记定时工具检查失败");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beijing_next_daily_is_stable_and_after_now() {
        assert_eq!(next_daily(0, 480, 9, 0), 3600);
        assert_eq!(next_daily(3600, 480, 9, 0), 90000);
        assert_eq!(next_daily(3599, 480, 9, 0), 3600);
    }
    #[test]
    fn only_real_quark_share_urls_are_accepted() {
        assert!(validate_quark_url("https://pan.quark.cn/s/abc123").is_ok());
        assert!(validate_quark_url("https://pan.quark.cn.evil.com/s/abc").is_err());
        assert!(validate_quark_url("http://pan.quark.cn/s/abc").is_err());
        assert!(validate_quark_url("https://pan.quark.cn@evil.test/s/x").is_err());
    }
    use crate::tools::{ToolOutput, ToolPolicy};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Probe {
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }
    #[async_trait]
    impl Tool for Probe {
        fn name(&self) -> &'static str {
            QUARK_INSPECT
        }
        fn description(&self) -> &'static str {
            "fixture share inspection"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"],"additionalProperties":false})
        }
        fn policy(&self) -> ToolPolicy {
            ToolPolicy {
                effect: ToolEffect::ReadOnly,
                ..Default::default()
            }
        }
        async fn call(&self, context: &ToolCtx, _: Value) -> Result<ToolOutput> {
            ensure!(context.chat_id == "scheduled", "wrong scope");
            self.calls.fetch_add(1, Ordering::SeqCst);
            ensure!(!self.fail.load(Ordering::SeqCst), "fixture network failure");
            Ok(ToolOutput {
                summary: "已读取真实 fixture 分享详情".into(),
                data: json!({}),
                artifacts: vec![],
            })
        }
    }
    #[tokio::test]
    async fn durable_schedule_honors_scopes_switches_leases_and_restart_without_duplication() {
        let root = std::env::temp_dir().join(format!(
            "schedule-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("yuantuan.db");
        let mut c = db::connect(&path).await.unwrap();
        db::migrate(&mut c).await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let tools = Registry::new();
        tools.register(Probe {
            calls: calls.clone(),
            fail: fail.clone(),
        });
        let id = register_quark(&path, &tools, "https://pan.quark.cn/s/test123", "测试")
            .await
            .unwrap();
        assert_eq!(
            register_quark(&path, &tools, "https://pan.quark.cn/s/test123", "改标题")
                .await
                .unwrap(),
            id
        );
        assert_eq!(list_quark(&path).await.unwrap().len(), 1);
        check_now(&path, &tools, id).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            list_quark(&path).await.unwrap()[0].last_status,
            "accessible"
        );
        fail.store(true, Ordering::SeqCst);
        check_now(&path, &tools, id).await.unwrap();
        assert_eq!(
            list_quark(&path).await.unwrap()[0].last_status,
            "check_failed"
        );
        capabilities::register(
            &path,
            QUARK_INSPECT,
            "tool",
            "probe",
            &tools.get(QUARK_INSPECT).unwrap().parameters_schema(),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE capability_catalog SET enabled=0 WHERE name=? AND kind='tool'")
            .bind(QUARK_INSPECT)
            .execute(&mut *c)
            .await
            .unwrap();
        check_now(&path, &tools, id).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        sqlx::query("UPDATE capability_catalog SET enabled=1,scopes='[\"group:42\"]' WHERE name=? AND kind='tool'").bind(QUARK_INSPECT).execute(&mut *c).await.unwrap();
        assert!(
            register_quark(&path, &tools, "https://pan.quark.cn/s/test456", "")
                .await
                .is_err()
        );
        sqlx::query("UPDATE capability_catalog SET scopes='[\"private:scheduled\"]' WHERE name=? AND kind='tool'").bind(QUARK_INSPECT).execute(&mut *c).await.unwrap();
        fail.store(false, Ordering::SeqCst);
        sqlx::query("UPDATE scheduled_tools SET next_run_at=0,last_status='checking',claimed_at=? WHERE id=?").bind(now()).bind(id).execute(&mut *c).await.unwrap();
        run_due(&path, &tools).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        sqlx::query("UPDATE scheduled_tools SET claimed_at=? WHERE id=?")
            .bind(now() - LEASE_SECS - 1)
            .bind(id)
            .execute(&mut *c)
            .await
            .unwrap();
        drop(c); // reopen through the normal DB path, as after process restoration
        run_due(&path, &tools).await.unwrap();
        run_due(&path, &tools).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(toggle(&path, id, false).await.unwrap());
        assert!(check_now(&path, &tools, id).await.is_err());
        assert!(remove(&path, id).await.unwrap());
        assert!(list_quark(&path).await.unwrap().is_empty());
        assert!(register_task(&path, &tools, "tencent-docs.call", "", "写入", "08:30").await.is_err());
        let id = register_task(&path, &tools, QUARK_INSPECT, "https://pan.quark.cn/s/test789", "自定义", "19:30")
            .await.unwrap();
        let jobs = list_tasks(&path, None).await.unwrap();
        assert_eq!((jobs[0].local_hour, jobs[0].local_minute), (19, 30));
        assert_eq!(list_quark(&path).await.unwrap()[0].id, id);
        let mut c = db::connect(&path).await.unwrap();
        sqlx::query("UPDATE scheduled_tools SET last_status='checking',claimed_at=? WHERE id=?")
            .bind(now()).bind(id).execute(&mut *c).await.unwrap();
        assert!(!set_task_time(&path, id, "18:00").await.unwrap());
        assert!(!remove_task(&path, id, None).await.unwrap());
        drop(c);
        if let Ok(database) = db::database(&path) {
            database.close().await;
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
