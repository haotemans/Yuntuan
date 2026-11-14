//! Immediate, source-backed working context is separate from a person's portrait.
use crate::{
    db::{params, SqliteExt},
    event::MessageReceivedPayload,
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub key: String,
    pub quote: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub context_id: Option<i64>,
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub goal_quote: String,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

pub async fn view(
    c: &mut sqlx::SqliteConnection,
    m: &MessageReceivedPayload,
    cutoff: i64,
) -> Result<Vec<Value>> {
    let rows=sqlx::query("SELECT v.* FROM working_context_versions v WHERE chat_id=?1 AND chat_type=?2 AND owner_pid=?3 AND source_msg_id<=?4 AND source_ts BETWEEN ?5-2592000 AND ?5 AND revision=(SELECT MAX(h.revision) FROM working_context_versions h WHERE h.context_id=v.context_id AND h.source_msg_id<=?4) ORDER BY source_msg_id DESC LIMIT 3")
        .bind(&m.chat_id).bind(&m.chat_type).bind(&m.sender_pid).bind(cutoff).bind(m.ts).fetch_all(c).await?;
    rows.iter().map(|r|Ok(json!({"context_id":r.try_get::<i64,_>("context_id")?,"revision":r.try_get::<i64,_>("revision")?,"owner_pid":m.sender_pid,"source_msg_id":r.try_get::<i64,_>("source_msg_id")?,"source_ts":r.try_get::<i64,_>("source_ts")?,"goal":{"quote":r.try_get::<String,_>("goal_quote")?,"source_msg_id":r.try_get::<i64,_>("goal_source_id")?},"conditions":serde_json::from_str::<Value>(&r.try_get::<String,_>("conditions")?)?,"kind":"user_stated_working_context","execution_authorization":false}))).collect()
}

fn valid_quote(text: &str, quote: &str) -> bool {
    (2..=400).contains(&quote.chars().count()) && text.contains(quote)
}

pub async fn apply(path: &Path, m: &MessageReceivedPayload, update: &Update) -> Result<i64> {
    ensure!(
        m.msg_id > 0 && m.sender_pid != "self" && !m.sender_bot,
        "工作记忆来源无效"
    );
    ensure!(
        !update.goal_quote.is_empty() || !update.conditions.is_empty(),
        "没有需要更新的工作条件"
    );
    ensure!(update.conditions.len() <= 12, "本轮条件过多");
    ensure!(
        update.goal_quote.is_empty() || valid_quote(&m.text, &update.goal_quote),
        "目标缺少当前原话"
    );
    for condition in &update.conditions {
        ensure!(
            !condition.key.trim().is_empty()
                && condition.key.chars().count() <= 40
                && !condition.key.chars().any(char::is_control)
                && valid_quote(&m.text, &condition.quote),
            "条件缺少有效当前原话"
        );
    }
    let mut c = crate::db::connect(path).await?;
    let mut tx = c.begin_immediate().await?;
    let stored:Option<String>=sqlx::query_scalar("SELECT text FROM messages WHERE msg_id=?1 AND chat_id=?2 AND chat_type=?3 AND sender_pid=?4 AND sender_bot=0")
        .bind(m.msg_id).bind(&m.chat_id).bind(&m.chat_type).bind(&m.sender_pid).fetch_optional(&mut *tx).await?;
    ensure!(
        stored.as_deref() == Some(m.text.as_str()),
        "工作记忆与存储原话不一致"
    );
    let id = update.context_id.unwrap_or(m.msg_id);
    // Retries are idempotent and cannot rewrite an already accepted source event.
    let done:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM working_context_versions WHERE context_id=?1 AND source_msg_id=?2 AND chat_id=?3 AND chat_type=?4 AND owner_pid=?5)")
        .bind(id).bind(m.msg_id).bind(&m.chat_id).bind(&m.chat_type).bind(&m.sender_pid).fetch_one(&mut *tx).await?;
    if done != 0 {
        tx.commit().await?;
        return Ok(id);
    }
    let previous = sqlx::query(
        "SELECT * FROM working_context_versions WHERE context_id=?1 ORDER BY revision DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (revision, goal, goal_source, mut conditions) = if let Some(r) = previous {
        ensure!(
            update.context_id.is_some()
                && r.try_get::<String, _>("chat_id")? == m.chat_id
                && r.try_get::<String, _>("chat_type")? == m.chat_type
                && r.try_get::<String, _>("owner_pid")? == m.sender_pid,
            "工作记忆归属不匹配"
        );
        let revision: i64 = r.try_get("revision")?;
        ensure!(
            update.expected_revision == Some(revision)
                && r.try_get::<i64, _>("source_msg_id")? < m.msg_id,
            "工作记忆版本已变化"
        );
        (
            revision + 1,
            r.try_get::<String, _>("goal_quote")?,
            r.try_get::<i64, _>("goal_source_id")?,
            serde_json::from_str::<Vec<Value>>(&r.try_get::<String, _>("conditions")?)?,
        )
    } else {
        ensure!(
            update.context_id.is_none()
                && update.expected_revision.is_none()
                && !update.goal_quote.is_empty(),
            "新事项需要当前目标原话"
        );
        (1, update.goal_quote.clone(), m.msg_id, vec![])
    };
    for condition in &update.conditions {
        conditions.retain(|v| v["key"] != condition.key);
        conditions
            .push(json!({"key":condition.key,"quote":condition.quote,"source_msg_id":m.msg_id}));
    }
    ensure!(conditions.len() <= 24, "已保存条件达到上限，需要拆分事项");
    let (goal, goal_source) = if update.goal_quote.is_empty() {
        (goal, goal_source)
    } else {
        (update.goal_quote.clone(), m.msg_id)
    };
    let quote_chars: usize = conditions
        .iter()
        .filter_map(|v| v["quote"].as_str())
        .map(|v| v.chars().count())
        .sum();
    ensure!(
        goal.chars().count() + quote_chars <= 4000,
        "工作记忆原话达到长度上限，需要拆分事项"
    );
    tx.execute("INSERT INTO working_context_versions(context_id,revision,chat_id,chat_type,owner_pid,source_msg_id,source_ts,goal_quote,goal_source_id,conditions,recorded_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,unixepoch())",
        params![id,revision,m.chat_id,m.chat_type,m.sender_pid,m.msg_id,m.ts,goal,goal_source,serde_json::to_string(&conditions)?]).await?;
    tx.commit().await?;
    Ok(id)
}
