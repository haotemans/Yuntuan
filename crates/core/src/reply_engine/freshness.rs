//! Ordinary replies are perishable; durable task delivery has its own receipts.
use crate::{
    context_builder::ReplySnapshot, decision::DecisionOutput, event::MessageReceivedPayload,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyFreshness {
    pub cutoff: i64,
    pub expires_at: i64,
    pub social: bool,
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn fixture() -> PathBuf {
        let p = std::env::temp_dir().join(format!("yt-fresh-{}.db", rand::random::<u64>()));
        let mut c = crate::db::connect(&p).await.unwrap();
        crate::db::migrate(&mut c).await.unwrap();
        c.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('self',1,1); INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'42','group','a','云团这么早就睡了',unixepoch())").await.unwrap();
        p
    }
    fn guard() -> ReplyFreshness {
        ReplyFreshness {
            cutoff: 1,
            expires_at: crate::window::now() + 60,
            social: true,
        }
    }
    fn job(freshness: Option<ReplyFreshness>) -> ReplyJob {
        ReplyJob {
            freshness,
            chat_id: "42".into(),
            chat_type: ChatType::Group,
            target: 42,
            anchor_msg_id: 1,
            mention: false,
            mention_qq: None,
            kind: JobKind::Bubbles(vec![Bubble {
                text: "没睡".into(),
                at: false,
                meme: None,
            }]),
        }
    }

    #[tokio::test]
    async fn supersession_is_typed_and_does_not_cancel_own_following_bubbles() {
        let p = fixture().await;
        let g = guard();
        let mut c = crate::db::connect(&p).await.unwrap();
        c.execute_batch("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,at_me,reply_anchor_id) VALUES('42','private','a','新问题',unixepoch(),1,NULL),('42','group','a','普通插话',unixepoch(),0,NULL),('42','group','self','自己的第一泡',unixepoch(),0,1)").await.unwrap();
        assert!(g.reason(&p, "group", "42", 1).await.unwrap().is_none());
        c.execute_batch("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,at_me) VALUES('42','group','a','新问题',unixepoch(),1)").await.unwrap();
        assert!(g.reason(&p, "group", "42", 1).await.unwrap().is_some());
        let answer = ReplyFreshness {
            social: false,
            ..g.clone()
        };
        assert!(answer.reason(&p, "group", "42", 1).await.unwrap().is_none());
        let expired = ReplyFreshness {
            expires_at: crate::window::now() - 1,
            ..answer
        };
        assert!(expired
            .reason(&p, "group", "42", 1)
            .await
            .unwrap()
            .is_some());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE msg_id=1")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn queue_and_typing_checks_stop_old_social_but_unguarded_delivery_still_sends() {
        let p = fixture().await;
        let count = Arc::new(AtomicUsize::new(0));
        let captured = count.clone();
        let send: SendFn = Arc::new(move |_| {
            captured.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(json!({"message_id":42})) })
        });
        let engine = Arc::new(ReplyEngine::new(
            p.clone(),
            EventBus::default(),
            send,
            SelfMsgIds::default(),
            Arc::new(std::sync::RwLock::new(ReplyCfg {
                total_budget_ms: 100,
                first_delay_min_ms: 30,
                first_delay_max_ms: 30,
                ..ReplyCfg::default()
            })),
            MoodState::default(),
        ));
        process_job(
            &engine,
            job(Some(ReplyFreshness {
                expires_at: crate::window::now() - 1,
                ..guard()
            })),
        )
        .await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let queued = job(Some(guard()));
        assert!(can_send(&engine, &queued).await);
        let mut c = crate::db::connect(&p).await.unwrap();
        c.execute_batch("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,at_me) VALUES('42','group','a','新的定向消息',unixepoch(),1)").await.unwrap();
        let JobKind::Bubbles(bubbles) = &queued.kind else {
            unreachable!()
        };
        process_bubbles(&engine, &queued, bubbles).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        process_job(&engine, job(None)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_send_is_not_retried_after_a_new_directed_turn() {
        let p = fixture().await;
        let count = Arc::new(AtomicUsize::new(0));
        let captured = count.clone();
        let path = p.clone();
        let send: SendFn = Arc::new(move |_| {
            captured.fetch_add(1, Ordering::SeqCst);
            let path = path.clone();
            Box::pin(async move {
                let mut c = crate::db::connect(&path).await?;
                c.execute_batch("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,at_me) VALUES('42','group','a','换个问题',unixepoch(),1)").await?;
                anyhow::bail!("明确发送失败")
            })
        });
        let engine = Arc::new(ReplyEngine::new(
            p,
            EventBus::default(),
            send,
            SelfMsgIds::default(),
            Arc::new(std::sync::RwLock::new(ReplyCfg {
                total_budget_ms: 0,
                ..ReplyCfg::default()
            })),
            MoodState::default(),
        ));
        process_job(&engine, job(Some(guard()))).await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "新对话到来后不重试旧接话");
    }
}

impl ReplyFreshness {
    pub fn for_reply(
        snapshot: &ReplySnapshot,
        msg: &MessageReceivedPayload,
        out: &DecisionOutput,
    ) -> Self {
        let social = out.reply_mode == crate::engagement::ReplyMode::Social;
        Self {
            cutoff: snapshot.cutoff,
            expires_at: msg.ts.saturating_add(if social { 60 } else { 180 }),
            social,
        }
    }

    pub async fn reason(
        &self,
        path: &Path,
        kind: &str,
        chat: &str,
        anchor: i64,
    ) -> Result<Option<&'static str>> {
        if crate::window::now() > self.expires_at {
            return Ok(Some("回复已超过时效，原始消息仍保留"));
        }
        if !self.social {
            return Ok(None);
        }
        let mut c = crate::db::connect(path).await?;
        let replaced: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE chat_type=?1 AND chat_id=?2 AND msg_id>?3 AND ((sender_pid='self' AND COALESCE(reply_anchor_id,0)!=?4) OR (sender_pid!='self' AND sender_bot=0 AND (at_me=1 OR chat_type='private' OR ltrim(COALESCE(text,'')) LIKE '云团%'))))")
            .bind(kind).bind(chat).bind(self.cutoff).bind(anchor).fetch_one(&mut *c).await?;
        Ok((replaced != 0).then_some("旧接话已让位给后续定向对话或已发送回复"))
    }
}
