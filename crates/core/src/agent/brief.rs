//! Source-backed task requirements, separate from expendable execution history.
use crate::{
    db::{params, SqliteExt},
    event::MessageReceivedPayload,
};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use sqlx::{Row, SqliteConnection};

pub(crate) async fn source(
    c: &mut SqliteConnection,
    id: i64,
    actor: &str,
    chat: &str,
    kind: &str,
) -> Result<MessageReceivedPayload> {
    let r=sqlx::query("SELECT * FROM messages WHERE msg_id=?1 AND sender_pid=?2 AND chat_id=?3 AND chat_type=?4 AND sender_pid!='self' AND sender_bot=0")
        .bind(id).bind(actor).bind(chat).bind(kind).fetch_one(c).await?;
    Ok(MessageReceivedPayload {
        msg_id: id,
        chat_id: chat.into(),
        chat_type: kind.into(),
        sender_pid: actor.into(),
        text: r.try_get("text")?,
        at_me: r.try_get::<i64, _>("at_me")? != 0,
        reply_to: r.try_get("reply_to")?,
        has_image: r.try_get::<i64, _>("has_image")? != 0,
        sender_bot: false,
        image_urls: vec![],
        ts: r.try_get("ts")?,
    })
}
fn evidence(m: &MessageReceivedPayload) -> Value {
    json!({"msg_id":m.msg_id,"author_pid":m.sender_pid,"chat_id":m.chat_id,"chat_type":m.chat_type,"ts":m.ts,"text":clip(&m.text,4000),"text_truncated":m.text.chars().count()>4000})
}
pub(crate) async fn write(
    c: &mut SqliteConnection,
    id: &str,
    revision: i64,
    body: &Value,
    source: Option<i64>,
    event: &str,
) -> Result<()> {
    let raw = serde_json::to_string(body)?;
    ensure!(
        raw.len() <= 64_000,
        "任务说明已达到64KB上限，需要拆分任务，原要求未覆盖"
    );
    c.execute("INSERT INTO task_brief_versions(task_id,revision,body,source_msg_id,event_kind,created_at) VALUES(?1,?2,?3,?4,?5,unixepoch())",params![id,revision,raw,source,event]).await?;
    Ok(())
}
pub(crate) async fn initialize(
    c: &mut SqliteConnection,
    id: &str,
    goal: &str,
    actor: &str,
    chat: &str,
    kind: &str,
    anchor: Option<i64>,
    context: &Value,
) -> Result<()> {
    if !context["focus_context_id"].is_null() {
        let focus = context["focus_context_id"].as_i64();
        ensure!(
            anchor.is_some()
                && focus.is_some_and(|v| v > 0)
                && context["working_context"][0]["context_id"] == context["focus_context_id"],
            "选中的事项资料缺失，不能按不完整条件启动任务"
        );
    }
    let mut body = json!({"goal":goal,"initial_goal":goal,"origin":"router_handoff","request":null,"working_context":[],"changes":[]});
    if let Some(anchor) = anchor {
        let m = source(c, anchor, actor, chat, kind).await?;
        body["request"] = evidence(&m);
        if let Some(selected) = context["working_context"]
            .as_array()
            .and_then(|a| a.first())
        {
            let candidates = crate::working_context::view(c, &m, anchor).await?;
            let selected = candidates.into_iter().find(|v| {
                v["context_id"] == selected["context_id"] && v["revision"] == selected["revision"]
            });
            ensure!(selected.is_some(), "交接事项的归属或版本不匹配");
            body["working_context"] = json!([selected.unwrap()]);
        }
    }
    write(c, id, 1, &body, anchor, "create").await
}
/// Legacy tasks expose only their stored goal; missing historical provenance is not invented.
pub async fn current(c: &mut SqliteConnection, id: &str) -> Result<Value> {
    let r=sqlx::query("SELECT t.goal,t.created_by_pid,t.chat_id,r.chat_type,r.revision,b.body,b.revision AS brief_revision FROM tasks t JOIN task_runtime r USING(task_id) LEFT JOIN task_brief_versions b ON b.task_id=r.task_id AND b.revision=r.revision WHERE t.task_id=?")
        .bind(id).fetch_one(&mut *c).await?;
    let revision: i64 = r.try_get("revision")?;
    let raw: Option<String> = r.try_get("body")?;
    if raw.is_none() {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_brief_versions WHERE task_id=?)")
                .bind(id)
                .fetch_one(&mut *c)
                .await?;
        ensure!(!known, "当前任务说明版本缺失，停止执行以免使用旧要求");
    }
    let mut body = if let Some(raw) = raw {
        serde_json::from_str::<Value>(&raw)?
    } else {
        json!({"goal":r.try_get::<String,_>("goal")?,"origin":"legacy_task","request":null,"working_context":[],"changes":[]})
    };
    body["task_id"] = json!(id);
    body["revision"] = json!(revision);
    body["actor_pid"] = json!(r.try_get::<String, _>("created_by_pid")?);
    body["chat_id"] = json!(r.try_get::<String, _>("chat_id")?);
    body["chat_type"] = json!(r.try_get::<String, _>("chat_type")?);
    Ok(body)
}
pub(crate) async fn control(
    c: &mut SqliteConnection,
    id: &str,
    revision: i64,
    action: &str,
    text: &str,
    message: Option<&MessageReceivedPayload>,
    focus: Option<i64>,
) -> Result<()> {
    let mut body = current(c, id).await?;
    body["revision"] = json!(revision + 1);
    if action == "amend" {
        ensure!(
            !text.trim().is_empty() && text.chars().count() <= 4000,
            "补充要求必须为1至4000字"
        );
        body["changes"].as_array_mut().ok_or_else(||anyhow::anyhow!("任务变更记录格式无效"))?.push(json!({"text":text,"origin":if message.is_some(){"user_message"}else{"admin_control"},"source":message.map(evidence)}));
        if let (Some(m), Some(focus)) = (message, focus) {
            if let Some(existing) = body["working_context"].as_array().and_then(|a| a.first()) {
                ensure!(
                    existing["context_id"] == focus,
                    "不能将不同事项自动合并到原任务"
                );
            }
            let selected = crate::working_context::view(c, m, m.msg_id)
                .await?
                .into_iter()
                .find(|v| v["context_id"] == focus);
            ensure!(selected.is_some(), "任务补充的事项来源不可用");
            let selected = selected.unwrap();
            let previous_goal = &body["working_context"][0]["goal"]["quote"];
            if (previous_goal.is_string() && previous_goal != &selected["goal"]["quote"])
                || (previous_goal.is_null() && selected["goal"]["source_msg_id"] == m.msg_id)
            {
                body["goal"] = selected["goal"]["quote"].clone();
            }
            body["working_context"] = json!([selected]);
        }
    }
    write(
        c,
        id,
        revision + 1,
        &body,
        message.map(|m| m.msg_id),
        action,
    )
    .await?;
    c.execute(
        "UPDATE tasks SET goal=?2 WHERE task_id=?1",
        params![id, body["goal"].as_str().unwrap_or("")],
    )
    .await?;
    Ok(())
}
pub(crate) async fn inherit(
    c: &mut SqliteConnection,
    parent: &str,
    child: &str,
    goal: &str,
) -> Result<()> {
    let mut body = current(c, parent).await?;
    body["parent_task_id"] = json!(parent);
    body["parent_revision"] = body["revision"].clone();
    body["parent_goal"] = body["goal"].clone();
    body["goal"] = json!(goal);
    body["initial_goal"] = json!(goal);
    for key in ["task_id", "revision"] {
        body.as_object_mut().unwrap().remove(key);
    }
    write(c, child, 1, &body, None, "delegate").await
}

fn clip(v: &str, n: usize) -> String {
    v.chars().take(n).collect()
}
fn visible(body: &Value, cutoff: i64) -> bool {
    body["request"]["msg_id"]
        .as_i64()
        .is_none_or(|id| id <= cutoff)
        && body["changes"].as_array().is_none_or(|a| {
            a.iter()
                .all(|v| v["source"]["msg_id"].as_i64().is_none_or(|id| id <= cutoff))
        })
}
/// Compact Router candidates; full requirements are loaded only for the selected task.
pub async fn candidates(
    c: &mut SqliteConnection,
    m: &MessageReceivedPayload,
    cutoff: i64,
) -> Result<Vec<Value>> {
    let rows=sqlx::query("SELECT t.task_id,t.goal,t.used_calls,r.phase,r.revision,r.delivery_state,r.final_text,r.stop_reason FROM tasks t JOIN task_runtime r USING(task_id) WHERE t.created_by_pid=?1 AND t.chat_id=?2 AND r.chat_type=?3 AND r.parent_id IS NULL AND t.created_at<=?4 AND (t.state='running' OR COALESCE(t.finished_at,t.created_at)>=?4-2592000) ORDER BY r.updated_at DESC,t.task_id DESC LIMIT 6")
        .bind(&m.sender_pid).bind(&m.chat_id).bind(&m.chat_type).bind(m.ts).fetch_all(&mut *c).await?;
    let mut out = vec![];
    for r in rows {
        let id: String = r.try_get("task_id")?;
        let b = current(c, &id).await?;
        if !visible(&b, cutoff) {
            continue;
        }
        out.push(json!({"task_id":id,"goal":clip(&r.try_get::<String,_>("goal")?,240),"revision":r.try_get::<i64,_>("revision")?,"phase":r.try_get::<String,_>("phase")?,"delivery_state":r.try_get::<String,_>("delivery_state")?,"used_calls":r.try_get::<i64,_>("used_calls")?,"working_context_id":b["working_context"][0]["context_id"],"working_context_revision":b["working_context"][0]["revision"],"last_change":b["changes"].as_array().and_then(|a|a.last()).map(|v|clip(v["text"].as_str().unwrap_or(""),240)),"result_preview":r.try_get::<Option<String>,_>("final_text")?.map(|s|clip(&s,600)),"stop_reason":r.try_get::<Option<String>,_>("stop_reason")?.map(|s|clip(&s,240))}));
        if out.len() == 3 {
            break;
        }
    }
    Ok(out)
}
pub async fn selected(
    c: &mut SqliteConnection,
    m: &MessageReceivedPayload,
    cutoff: i64,
    id: &str,
) -> Result<Value> {
    let mut tx = c.transaction().await?;
    let view = selected_inner(&mut tx, m, cutoff, id).await?;
    tx.commit().await?;
    Ok(view)
}
async fn selected_inner(
    c: &mut SqliteConnection,
    m: &MessageReceivedPayload,
    cutoff: i64,
    id: &str,
) -> Result<Value> {
    let permitted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks t JOIN task_runtime r USING(task_id) WHERE t.task_id=?1 AND t.created_by_pid=?2 AND t.chat_id=?3 AND r.chat_type=?4 AND r.parent_id IS NULL AND (t.created_at<=?5 OR r.anchor_id=?6))")
        .bind(id).bind(&m.sender_pid).bind(&m.chat_id).bind(&m.chat_type).bind(m.ts).bind(m.msg_id).fetch_one(&mut *c).await?;
    ensure!(permitted, "任务资料归属不匹配");
    let body = current(c, id).await?;
    ensure!(visible(&body, cutoff), "任务资料超出当前消息上界");
    let r = sqlx::query(
        "SELECT phase,final_text,stop_reason,delivery_state FROM task_runtime WHERE task_id=?",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let rows=sqlx::query("SELECT i.id,i.task_id,i.tool_name,i.revision,r.revision AS current_revision,i.state,substr(i.result,1,700) AS result_preview FROM task_invocations i JOIN task_runtime r ON r.task_id=i.task_id WHERE r.root_id=? ORDER BY i.started_at DESC,i.id DESC LIMIT 6").bind(id).fetch_all(&mut *c).await?;
    let receipts:Vec<Value>=rows.iter().map(|r|Ok(json!({"receipt_id":r.try_get::<String,_>("id")?,"tool":r.try_get::<String,_>("tool_name")?,"revision":r.try_get::<i64,_>("revision")?,"matches_current_revision":r.try_get::<i64,_>("revision")?==r.try_get::<i64,_>("current_revision")?,"task_id":r.try_get::<String,_>("task_id")?,"state":r.try_get::<String,_>("state")?,"result_preview":r.try_get::<Option<String>,_>("result_preview")?}))).collect::<Result<_>>()?;
    Ok(
        json!({"task_id":id,"revision":body["revision"],"brief":body,"phase":r.try_get::<String,_>("phase")?,"delivery_state":r.try_get::<String,_>("delivery_state")?,"result_preview":r.try_get::<Option<String>,_>("final_text")?.map(|s|clip(&s,2400)),"stop_reason":r.try_get::<Option<String>,_>("stop_reason")?.map(|s|clip(&s,600)),"invocations":receipts,"previews_may_be_truncated":true}),
    )
}
