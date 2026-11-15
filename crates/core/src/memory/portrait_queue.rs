//! Exact portrait coverage and bounded recent/backfill selection; never skips an unseen gap.
use super::processing::{self, Batch, Scope};
use crate::db::{params, SqliteExt};
use anyhow::{ensure, Result};
use serde::Serialize;
use sqlx::{sqlite::SqliteRow, Row};

#[derive(Serialize)]
pub(super) struct Source {
    pub msg_id: i64,
    pub chat_id: String,
    pub chat_type: String,
    pub text: String,
    pub ts: i64,
    pub text_truncated: bool,
}

pub(super) struct Window {
    pub sources: Vec<Source>,
    pub inspected: Vec<i64>,
    pub visibility: Option<(String, String)>,
}

pub(super) async fn load(conn: &mut sqlx::SqliteConnection, person: &str) -> Result<Window> {
    let cursor = processing::cursor(conn, &Scope::portrait(person)).await?;
    let query = "SELECT m.msg_id,m.chat_id,m.chat_type,m.text,m.ts FROM messages m
      WHERE m.sender_pid=?1 AND m.chat_type IN ('group','private') AND m.msg_id>?2
      AND length(trim(COALESCE(m.text,'')))>=4
      AND NOT EXISTS(SELECT 1 FROM portrait_coverage c WHERE c.person_id=?1 AND c.msg_id=m.msg_id)";
    // Recent messages get 120 of the same 160-message budget; remaining slots close old gaps.
    let mut rows = sqlx::query(&format!(
        "{query} AND m.ts>=?3 ORDER BY m.msg_id DESC LIMIT 120"
    ))
    .bind(person)
    .bind(cursor)
    .bind(processing::now() - 30 * 86400)
    .fetch_all(&mut *conn)
    .await?;
    let visibility = rows.first().map(scope).transpose()?;
    let oldest = sqlx::query(&format!("{query} ORDER BY m.msg_id LIMIT 160"))
        .bind(person)
        .bind(cursor)
        .fetch_all(&mut *conn)
        .await?;
    let visibility = visibility.or(oldest.first().map(scope).transpose()?);
    rows.retain(|r| visibility.as_ref().is_some_and(|v| same_scope(r, v)));
    let recent: std::collections::BTreeSet<i64> = rows.iter().map(|r| r.get("msg_id")).collect();
    for row in oldest {
        if rows.len() >= 160 {
            break;
        }
        if visibility.as_ref().is_some_and(|v| same_scope(&row, v))
            && !rows
                .iter()
                .any(|r| r.get::<i64, _>("msg_id") == row.get::<i64, _>("msg_id"))
        {
            rows.push(row);
        }
    }
    // Retain recent priority under the character budget, then present evidence chronologically.
    let mut sources = Vec::new();
    let mut inspected = Vec::new();
    let mut chars = 0;
    let has_backfill = rows
        .iter()
        .any(|r| !recent.contains(&r.get::<i64, _>("msg_id")));
    let mut recent_chars = 0;
    for row in rows {
        let original: String = row.try_get("text")?;
        let id = row.try_get("msg_id")?;
        if crate::consolidation::is_sensitive(&original) {
            inspected.push(id);
            continue;
        }
        let text: String = original.chars().take(4000).collect();
        if has_backfill && recent.contains(&id) && recent_chars + text.chars().count() > 24_000 {
            continue;
        }
        if chars + text.chars().count() > 32_000 {
            continue;
        }
        chars += text.chars().count();
        if recent.contains(&id) {
            recent_chars += text.chars().count();
        }
        inspected.push(id);
        sources.push(Source {
            msg_id: id,
            chat_id: row.try_get("chat_id")?,
            chat_type: row.try_get("chat_type")?,
            text,
            ts: row.try_get("ts")?,
            text_truncated: original.chars().count() > 4000,
        });
    }
    sources.sort_by_key(|s| s.msg_id);
    inspected.sort_unstable();
    Ok(Window {
        sources,
        inspected,
        visibility,
    })
}

fn scope(r: &SqliteRow) -> sqlx::Result<(String, String)> {
    Ok((r.try_get("chat_type")?, r.try_get("chat_id")?))
}
fn same_scope(r: &SqliteRow, v: &(String, String)) -> bool {
    r.get::<String, _>("chat_type") == v.0
        && (v.0 != "private" || r.get::<String, _>("chat_id") == v.1)
}

/// Called in the same transaction as verified outputs and final run status.
pub(super) async fn complete(
    conn: &mut sqlx::SqliteConnection,
    batch: &Batch,
    ids: &[i64],
) -> Result<()> {
    let at = processing::now();
    let changed = conn.execute("UPDATE memory_batches SET status='completed',finished_at=?2 WHERE id=?1 AND status='running' AND lease_until>?2",params![batch.id,at]).await?;
    ensure!(changed == 1, "归纳批次已失效，拒绝迟到结果");
    for id in ids {
        conn.execute("INSERT OR IGNORE INTO portrait_coverage(person_id,msg_id,batch_id,processed_at) VALUES(?1,?2,?3,?4)",params![batch.scope.owner_id,id,batch.id,at]).await?;
    }
    // Only the contiguous frontier advances; disjoint recent ranges remain in coverage.
    conn.execute("UPDATE memory_progress SET cursor=MAX(cursor,COALESCE(
       (SELECT MIN(m.msg_id)-1 FROM messages m WHERE m.sender_pid=?1 AND m.msg_id>memory_progress.cursor
        AND m.chat_type IN ('group','private') AND length(trim(COALESCE(m.text,'')))>=4
        AND NOT EXISTS(SELECT 1 FROM portrait_coverage c WHERE c.person_id=?1 AND c.msg_id=m.msg_id)),
       (SELECT MAX(m.msg_id) FROM messages m WHERE m.sender_pid=?1 AND m.chat_type IN ('group','private') AND length(trim(COALESCE(m.text,'')))>=4),cursor)),updated_at=?2
       WHERE stage='portrait' AND owner_id=?1 AND chat_type=''",params![batch.scope.owner_id,at]).await?;
    Ok(())
}
