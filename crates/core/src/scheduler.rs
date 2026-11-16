//! Persisted daily scheduling of read-only tools. No model required for recurring jobs.
//! Each invocation goes through the already-registered HostedTool and capability policy.
use crate::{capabilities, db, tools::{Registry, ToolCtx, ToolEffect}};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{path::{Path, PathBuf}, time::{Duration, SystemTime, UNIX_EPOCH}};

pub const QUARK_INSPECT: &str = "quark-drive.inspect_share";
const MAX_DUE_PER_TICK: i64 = 10;
const LEASE_SECS: i64 = 300;

#[derive(Clone, Debug, Serialize)]
pub struct ScheduledItem {
    pub id: i64,
    pub title: String,
    pub tool_name: String,
    pub resource_key: String,
    pub enabled: bool,
    pub next_run_at: i64,
    pub last_run_at: Option<i64>,
    pub last_status: String,
    pub last_summary: String,
}
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}
fn ctx() -> ToolCtx {
    ToolCtx { task_id:None, chat_id:"scheduled".into(), chat_type:"private".into(), sender_pid:"scheduler".into(), locale:Some("zh-CN".into()) }
}
pub fn next_daily(now: i64, offset_minutes: i64, hour: i64, minute: i64) -> i64 {
    let offset=offset_minutes * 60;
    let day=(now+offset).div_euclid(86_400)*86_400;
    let target=day+hour*3600+minute*60-offset;
    if target>now { target } else { target+86_400 }
}
pub fn validate_quark_url(url: &str) -> Result<String> {
    let url=url.trim();
    ensure!(url.len()<=1024 && !url.contains(char::is_whitespace), "链接格式无效");
    let parsed=reqwest::Url::parse(url).context("夸克分享 URL 无效")?;
    ensure!(parsed.scheme()=="https" && parsed.host_str()==Some("pan.quark.cn")
        && parsed.username().is_empty() && parsed.password().is_none()
        && parsed.path().starts_with("/s/") && parsed.path().len()>3
        && parsed.fragment().is_none(), "仅接受夸克网盘官方 HTTPS 分享链接");
    Ok(url.to_owned())
}
pub async fn list_quark(path:&Path) -> Result<Vec<ScheduledItem>> {
    let mut c=db::connect(path).await?;
    let rows=sqlx::query("SELECT id,title,tool_name,resource_key,enabled,next_run_at,last_run_at,last_status,last_summary FROM scheduled_tools WHERE tool_name=? ORDER BY id DESC LIMIT 200")
        .bind(QUARK_INSPECT).fetch_all(&mut *c).await?;
    rows.into_iter().map(|row|Ok(ScheduledItem {
        id:row.try_get("id")?,
        title:row.try_get("title")?,
        tool_name:row.try_get("tool_name")?,
        resource_key:row.try_get("resource_key")?,
        enabled:row.try_get::<i64,_>("enabled")? != 0,
        next_run_at:row.try_get("next_run_at")?,
        last_run_at:row.try_get("last_run_at")?,
        last_status:row.try_get("last_status")?,
        last_summary:row.try_get("last_summary")?,
    })).collect()
}
async fn ensure_readonly(path:&Path,tools:&Registry,name:&str, args:&Value)->Result<()> {
    let tool=tools.get(name).context("对应插件未启用或尚未完成授权配置")?;
    ensure!(tool.policy().effect==ToolEffect::ReadOnly, "定时任务首期只允许只读 Tool");
    ensure!(tool.available_in_chat(&ctx()), "当前工具不对调度器开放");
    capabilities::check_tool(path,name,&ctx(),&tool.parameters_schema(),args).await
}

pub async fn register_quark(path:&Path,tools:&Registry,url:&str,title:&str)->Result<i64> {
    let url=validate_quark_url(url)?;
    let label=title.trim();
    ensure!(label.chars().count()<=100, "任务标题过长");
    let args=json!({"url":url});
    ensure_readonly(path,tools,QUARK_INSPECT,&args).await?;
    let mut c=db::connect(path).await?;
    let due=next_daily(now(),480,9,0);
    let row=sqlx::query("INSERT INTO scheduled_tools(title,tool_name,resource_key,arguments_json,timezone_offset_minutes,local_hour,local_minute,next_run_at) VALUES(?,?,?,?,480,9,0,?) ON CONFLICT(tool_name,resource_key) DO UPDATE SET title=excluded.title,enabled=1 RETURNING id")
        .bind(if label.is_empty(){"夸克分享链接"}else{label})
        .bind(QUARK_INSPECT).bind(&url).bind(args.to_string()).bind(due)
        .fetch_one(&mut *c).await?;
    Ok(row.try_get("id")?)
}
pub async fn toggle(path:&Path,id:i64,enabled:bool)->Result<bool> {
    ensure!(id>0,"任务编号无效");
    let mut c=db::connect(path).await?;
    let n=sqlx::query("UPDATE scheduled_tools SET enabled=? WHERE id=? AND tool_name=?")
        .bind(enabled).bind(id).bind(QUARK_INSPECT).execute(&mut *c).await?.rows_affected();
    Ok(n==1)
}
pub async fn remove(path:&Path,id:i64)->Result<bool> {
    ensure!(id>0,"任务编号无效");
    let mut c=db::connect(path).await?;
    let n=sqlx::query("DELETE FROM scheduled_tools WHERE id=? AND tool_name=?")
        .bind(id).bind(QUARK_INSPECT).execute(&mut *c).await?.rows_affected();
    Ok(n==1)
}

async fn execute_one(path:&Path,tools:&Registry,id:i64,force:bool)->Result<()> {
    let started=now();
    let mut c=db::connect(path).await?;
    let claimed=sqlx::query("UPDATE scheduled_tools SET last_status='checking',claimed_at=? WHERE id=? AND enabled=1 AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?) AND (?=1 OR next_run_at<=?)")
        .bind(started).bind(id).bind(started-LEASE_SECS)
        .bind(i64::from(force)).bind(started).execute(&mut *c).await?.rows_affected();
    if claimed==0 {
        bail!("任务未到执行时间、已停用或正在运行");
    }
    let row=sqlx::query("SELECT tool_name,arguments_json,timezone_offset_minutes,local_hour,local_minute FROM scheduled_tools WHERE id=?")
        .bind(id).fetch_one(&mut *c).await?;
    let tool_name:String=row.try_get("tool_name")?;
    let args:String=row.try_get("arguments_json")?;
    let offset:i64=row.try_get("timezone_offset_minutes")?;
    let hour:i64=row.try_get("local_hour")?;
    let minute:i64=row.try_get("local_minute")?;
    drop(c);
    let work=async {
        let args:Value=serde_json::from_str(&args).context("已登记的定时任务参数损坏")?;
        ensure_readonly(path,tools,&tool_name,&args).await?;
        let tool=tools.get(&tool_name).context("对应插件已卸载或未启用")?;
        let out=tokio::time::timeout(Duration::from_secs(120),tool.call(&ctx(),args))
            .await.context("执行超时")??;
        // Don't persist private file listings; only an actual success receipt.
        Ok::<String,anyhow::Error>(out.summary.chars().take(400).collect())
    }.await;
    let (status,summary)=match work {
        Ok(message)=>("accessible",message),
        Err(error)=>("check_failed",format!("检查失败：{}",error.to_string().chars().take(200).collect::<String>())),
    };
    let finished=now();
    let next=next_daily(finished,offset,hour,minute);
    let mut c=db::connect(path).await?;
    let n=sqlx::query("UPDATE scheduled_tools SET last_run_at=?,last_status=?,last_summary=?,next_run_at=?,claimed_at=NULL WHERE id=? AND claimed_at=?")
        .bind(finished).bind(status).bind(summary).bind(next).bind(id).bind(started)
        .execute(&mut *c).await?.rows_affected();
    ensure!(n==1,"定时任务完成时状态已改变，结果未覆盖新任务");
    Ok(())
}
pub async fn check_now(path:&Path,tools:&Registry,id:i64)->Result<()> {
    ensure!(id>0,"任务编号无效");
    execute_one(path,tools,id,true).await
}
pub fn spawn(path:PathBuf,tools:Registry)->tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval=tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error)=run_due(&path,&tools).await {
                tracing::warn!(error=%error,"每日只读工具调度失败");
            }
        }
    })
}
async fn run_due(path:&Path,tools:&Registry)->Result<()> {
    let time=now();
    let mut c=db::connect(path).await?;
    let ids:Vec<i64>=sqlx::query_scalar("SELECT id FROM scheduled_tools WHERE enabled=1 AND next_run_at<=? AND (last_status!='checking' OR claimed_at IS NULL OR claimed_at<=?) ORDER BY next_run_at,id LIMIT ?")
        .bind(time).bind(time-LEASE_SECS).bind(MAX_DUE_PER_TICK).fetch_all(&mut *c).await?;
    drop(c);
    for id in ids {
        if let Err(error)=execute_one(path,tools,id,false).await {
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
        assert_eq!(next_daily(0,480,9,0),3600);
        assert_eq!(next_daily(3600,480,9,0),90000);
        assert_eq!(next_daily(3599,480,9,0),3600);
    }
    #[test]
    fn only_real_quark_share_urls_are_accepted() {
        assert!(validate_quark_url("https://pan.quark.cn/s/abc123").is_ok());
        assert!(validate_quark_url("https://pan.quark.cn.evil.com/s/abc").is_err());
        assert!(validate_quark_url("http://pan.quark.cn/s/abc").is_err());
        assert!(validate_quark_url("https://pan.quark.cn@evil.test/s/x").is_err());
    }
}
