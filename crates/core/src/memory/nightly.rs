//! A bounded nightly queue. Attempts, per-person markers and spacing survive restarts.
use crate::{
    db::{self, params, SqliteExt},
    llm::Role,
};
use anyhow::Result;
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct Options {
    pub date: String,
    pub deadline: i64,
    pub limit: usize,
    pub interval_secs: u64,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

enum Claim {
    Person(String),
    Wait(u64),
    Done,
}

async fn claim(path: &Path, options: &Options, at: i64) -> Result<Claim> {
    if at >= options.deadline || options.limit == 0 {
        return Ok(Claim::Done);
    }
    let mut conn = db::connect(path).await?;
    let mut tx = conn.transaction().await?;
    let key = format!("profile_nightly:v1:{}", options.date);
    tx.execute("INSERT INTO state_kv(key,value,updated_at) VALUES(?1,'{\"used\":0,\"next_at\":0}',?2) ON CONFLICT(key) DO NOTHING",params![key,at]).await?;
    let stored: String = sqlx::query_scalar("SELECT value FROM state_kv WHERE key=?1")
        .bind(&key)
        .fetch_one(&mut *tx)
        .await?;
    let state: serde_json::Value = serde_json::from_str(&stored)?;
    let used = state["used"].as_u64().unwrap_or(0);
    if used >= options.limit.min(100) as u64 {
        return Ok(Claim::Done);
    }
    let next = state["next_at"].as_i64().unwrap_or(0);
    if next > at {
        return Ok(Claim::Wait((next - at) as u64));
    }
    let prefix = format!("profile_nightly_person:v1:{}:", options.date);
    let rotation = used >= options.limit.saturating_sub((options.limit / 5).max(1)) as u64;
    let person=sqlx::query_scalar::<_,String>(
        "WITH candidates AS (
            SELECT COALESCE(d.directory_id,m.sender_pid) AS subject,
                   MAX(m.ts) AS latest_ts,
                   MAX(COALESCE(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.attempt'),0)) AS last_attempt,
                   MAX(COALESCE(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.last_success_at'),json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.completed_at'),0)) AS last_success
            FROM messages m
            LEFT JOIN memory_directory d ON d.person_id=m.sender_pid
            LEFT JOIN state_kv s ON s.key='profile_refresh:v1:'||COALESCE(d.directory_id,m.sender_pid)
            LEFT JOIN memory_progress progress ON progress.stage='portrait' AND progress.owner_id=m.sender_pid AND progress.chat_type=''
            WHERE m.sender_pid<>'self' AND COALESCE(d.directory_id,m.sender_pid)<>'self'
              AND m.chat_type IN ('group','private')
              AND length(trim(COALESCE(m.text,'')))>=4
              AND m.msg_id>COALESCE(progress.cursor,0)
              AND NOT EXISTS(SELECT 1 FROM portrait_coverage c WHERE c.person_id=m.sender_pid AND c.msg_id=m.msg_id)
              AND NOT EXISTS(SELECT 1 FROM state_kv done WHERE done.key=?1||COALESCE(d.directory_id,m.sender_pid))
            GROUP BY COALESCE(d.directory_id,m.sender_pid)
            HAVING last_attempt<=?2-300
         )
         SELECT subject FROM candidates
         ORDER BY CASE WHEN ?3 THEN 0 ELSE CASE WHEN EXISTS(SELECT 1 FROM person_profile_versions v LEFT JOIN memory_progress vp ON vp.stage='portrait' AND vp.owner_id=v.person_id AND vp.chat_type='' WHERE v.person_id IN (SELECT candidates.subject UNION ALL SELECT alias FROM memory_aliases WHERE person_id=candidates.subject) AND v.source_msg_id>COALESCE(vp.cursor,0) AND NOT EXISTS(SELECT 1 FROM portrait_coverage c WHERE c.person_id=v.person_id AND c.msg_id=v.source_msg_id)) THEN 0 ELSE 1 END END,
         CASE WHEN ?3 THEN 0 ELSE CASE WHEN EXISTS(SELECT 1 FROM memory_facts f LEFT JOIN memory_progress fp ON fp.stage='portrait' AND fp.owner_id=f.owner_id AND fp.chat_type='' WHERE f.owner_type='person' AND f.owner_id IN (SELECT candidates.subject UNION ALL SELECT alias FROM memory_aliases WHERE person_id=candidates.subject) AND f.source_msg_id>COALESCE(fp.cursor,0)) THEN 0 ELSE 1 END END,
         CASE WHEN ?3 THEN 0 ELSE CASE WHEN latest_ts>=?2-86400 THEN 0 ELSE 1 END END,
         last_success,latest_ts DESC,subject LIMIT 1")
        .bind(prefix.clone()).bind(at).bind(rotation).fetch_optional(&mut *tx).await?;
    let Some(person) = person else {
        return Ok(Claim::Done);
    };
    // Compare-and-swap the shared daily row before committing the person's attempt.
    // Unknown/cancelled work holds the queue for five minutes plus the configured gap.
    let claimed = tx
        .execute(
            "UPDATE state_kv SET value=?3,updated_at=?4 WHERE key=?1 AND value=?2",
            params![
                key,
                stored,
                {
                    let mut state = state.clone();
                    state["used"] = serde_json::json!(used + 1);
                    state["next_at"] =
                        serde_json::json!(at + 300 + options.interval_secs.min(3600) as i64);
                    state["active_person"] = serde_json::json!(person);
                    state.to_string()
                },
                at
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(Claim::Wait(1));
    }
    tx.execute(
        "INSERT INTO state_kv(key,value,updated_at) VALUES(?1,'started',?2)",
        params![format!("{prefix}{person}"), at],
    )
    .await?;
    tx.commit().await?;
    Ok(Claim::Person(person))
}

pub async fn run(path: &Path, slot: &crate::bot::SharedLlm, options: &Options) -> Result<()> {
    let _guard = RUNNING.lock().await;
    loop {
        // Resolve after each gap, never retain a startup snapshot across the night.
        let Some(gw) = slot.read().unwrap().clone() else {
            return Ok(());
        };
        let Some(role) = gw.role(Role::Decision) else {
            return Ok(());
        };
        let binding = format!(
            "{}:{}:{:?}",
            role.provider, role.model, role.reasoning_effort
        );
        let mut conn = db::connect(path).await?;
        let circuit: Option<String> = sqlx::query_scalar("SELECT value FROM state_kv WHERE key=?1")
            .bind(format!("profile_nightly:v1:{}", options.date))
            .fetch_optional(&mut *conn)
            .await?;
        let circuit: serde_json::Value = circuit
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if circuit["failure_streak"].as_u64().unwrap_or(0) >= 3
            && circuit["failure_binding"].as_str() == Some(binding.as_str())
        {
            return Ok(());
        }
        drop(conn);
        match claim(path, options, now()).await? {
            Claim::Done => return Ok(()),
            Claim::Wait(seconds) => {
                let remaining = (options.deadline - now()).max(0) as u64;
                if remaining == 0 {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(seconds.min(remaining))).await;
            }
            Claim::Person(person) => {
                let result = super::refresh::person(path, &gw, &person, false).await;
                if result.is_err() {
                    tracing::warn!(person_id=%person,"夜间画像整理失败或超时，保留现有资料");
                }
                let at = now();
                let mut conn = db::connect(path).await?;
                let error: Option<String> = sqlx::query_scalar(
                    "SELECT json_extract(value,'$.error_kind') FROM state_kv WHERE key=?1",
                )
                .bind(format!("profile_refresh:v1:{person}"))
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
                let error = if result.is_err() {
                    error.or_else(|| Some("storage_failed".into()))
                } else {
                    None
                };
                let streak = if error.is_some()
                    && circuit["failure_binding"].as_str() == Some(binding.as_str())
                    && circuit["failure_kind"].as_str() == error.as_deref()
                {
                    circuit["failure_streak"].as_u64().unwrap_or(0) + 1
                } else if error.is_some() {
                    1
                } else {
                    0
                };
                conn.execute("UPDATE state_kv SET value=json_patch(value,?2) WHERE key=?1",params![format!("profile_nightly:v1:{}",options.date),serde_json::json!({"failure_streak":streak,"failure_binding":binding,"failure_kind":error}).to_string()]).await?;
                conn.execute(
                    "UPDATE state_kv SET value=?2,updated_at=?3 WHERE key=?1",
                    params![
                        format!("profile_nightly_person:v1:{}:{person}", options.date),
                        match &result {
                            Ok(r) => r.status,
                            Err(_) => "failed",
                        },
                        at
                    ],
                )
                .await?;
                conn.execute("UPDATE state_kv SET value=json_set(value,'$.next_at',?3,'$.active_person',NULL),updated_at=?4 WHERE key=?1 AND json_extract(value,'$.active_person')=?2",
                    params![format!("profile_nightly:v1:{}",options.date),person,at+options.interval_secs.min(3600) as i64,at]).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn claim_persists_spacing_quota_and_person_deduplication() {
        let path = std::env::temp_dir().join(format!("nightly-claim-{}.db", rand::random::<u64>()));
        let mut conn = db::connect(&path).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
        conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1);
          INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','我长期使用 Rust',1),('p','private','b','我长期使用 Python',2);").await.unwrap();
        let options = Options {
            date: "2026-11-06".into(),
            deadline: 5000,
            limit: 1,
            interval_secs: 60,
        };
        let Claim::Person(first) = claim(&path, &options, 1000).await.unwrap() else {
            panic!("missing first person")
        };
        assert_eq!(first, "b");
        // Raising the cap cannot defeat persisted spacing or select b a second time.
        let more = Options {
            limit: 2,
            ..options
        };
        assert!(matches!(
            claim(&path, &more, 1100).await.unwrap(),
            Claim::Wait(260)
        ));
        assert!(matches!(claim(&path,&more,1360).await.unwrap(),Claim::Person(p) if p=="a"));
        assert!(matches!(
            claim(&path, &more, 2000).await.unwrap(),
            Claim::Done
        ));
        assert!(matches!(
            claim(&path, &more, 5000).await.unwrap(),
            Claim::Done
        ));
    }

    #[tokio::test]
    async fn rotation_uses_last_success_not_last_failed_attempt() {
        let path =
            std::env::temp_dir().join(format!("nightly-priority-{}.db", rand::random::<u64>()));
        let mut conn = db::connect(&path).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
        conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('new',1,1),('old',1,1);
          INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','new','我最近在做新项目',1000),('g','group','old','长期没有整理的发言',1);
          INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at) VALUES('new','ongoing_projects','新项目',1,'我最近在做新项目',1000);
          INSERT INTO state_kv(key,value,updated_at) VALUES('profile_refresh:v1:old','{\"attempt\":100,\"status\":\"failed\",\"completed_at\":1}',999),('profile_refresh:v1:new','{\"attempt\":100,\"completed_at\":900}',900);").await.unwrap();
        let options = Options {
            date: "test".into(),
            deadline: 5000,
            limit: 10,
            interval_secs: 60,
        };
        assert!(matches!(claim(&path,&options,1000).await.unwrap(),Claim::Person(p) if p=="new"));
        conn.execute_batch("DELETE FROM state_kv WHERE key LIKE 'profile_nightly_person:%'; UPDATE state_kv SET value='{\"used\":8,\"next_at\":0}' WHERE key='profile_nightly:v1:test';").await.unwrap();
        assert!(matches!(claim(&path,&options,1000).await.unwrap(),Claim::Person(p) if p=="old"));
    }
}
