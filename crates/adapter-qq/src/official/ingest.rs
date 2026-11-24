use super::OfficialQq;
use anyhow::Result;
use serde_json::{json, Value};
use yuantuan_core::{
    db::{self, SqliteExt},
    event::{Event, MessageReceivedPayload},
    platform::{official_person_id, valid_openid, QQ_OFFICIAL_ID, QQ_OFFICIAL_PREFIX},
};

fn image_url(attachment: &Value) -> Option<String> {
    let kind = attachment["content_type"].as_str()?;
    if !kind.starts_with("image/") {
        return None;
    }
    let raw = attachment["url"].as_str()?;
    if raw.len() > 2048 {
        return None;
    }
    let url = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_owned()
    };
    let parsed = reqwest::Url::parse(&url).ok()?;
    (parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port_or_known_default() == Some(443)
        && parsed
            .host_str()
            .is_some_and(|host| {
                host == "multimedia.nt.qq.com.cn"
                    || host.ends_with(".qq.com")
                    || host.ends_with(".qpic.cn")
            }))
    .then_some(url)
}

fn group_content<'a>(text: &'a str, is_self: impl FnOnce(&str) -> bool) -> &'a str {
    let text = text.trim();
    if let Some((mention, rest)) = text
        .strip_prefix("<@")
        .and_then(|text| text.split_once('>'))
    {
        if is_self(mention.trim_start_matches('!')) {
            return rest.trim_start();
        }
    }
    text
}

pub(super) async fn accept(adapter: &OfficialQq, event: &str, data: &Value) -> Result<()> {
    let (kind, sender, recipient) = match event {
        "GROUP_AT_MESSAGE_CREATE" | "GROUP_MESSAGE_CREATE" => (
            "group",
            data.pointer("/author/member_openid")
                .and_then(Value::as_str),
            data["group_openid"].as_str(),
        ),
        "C2C_MESSAGE_CREATE" => {
            let user = data.pointer("/author/user_openid").and_then(Value::as_str);
            ("private", user, user)
        }
        _ => return Ok(()),
    };
    let (Some(sender), Some(recipient), Some(key), Some(timestamp)) = (
        sender.filter(|id| valid_openid(id)),
        recipient.filter(|id| valid_openid(id)),
        data["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 512),
        data["timestamp"].as_str().filter(|ts| ts.len() <= 64),
    ) else {
        return Ok(());
    };
    if data.pointer("/author/bot").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let mut image_urls = Vec::new();
    for url in data["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(image_url)
        .take(3)
    {
        if !image_urls.contains(&url) {
            image_urls.push(url);
        }
    }
    let account = &adapter.0.cfg.app_id;
    let named_mentions = data["mentions"].as_array().into_iter().flatten().take(80);
    let is_self = |m: &Value| m["is_you"].as_bool() == Some(true) && m["scope"] != "all";
    let at_event = event == "GROUP_AT_MESSAGE_CREATE";
    let at_me = at_event || (kind == "group" && named_mentions.clone().any(is_self));
    let mut mentions = Vec::new();
    if at_me {
        mentions.push("self".to_owned());
    }
    if kind == "group" {
        for mention in named_mentions.filter(|mention| !is_self(mention)) {
            let id = if mention["scope"] == "all" {
                Some("all".to_owned())
            } else {
                mention["member_openid"]
                    .as_str()
                    .filter(|id| valid_openid(id))
                    .map(|id| official_person_id(account, id))
            };
            if let Some(id) = id.filter(|id| !mentions.contains(id)) {
                mentions.push(id);
            }
        }
    }
    let text = data["content"].as_str().unwrap_or_default();
    let text: String = if at_me {
        group_content(text, |id| {
            if let Some(mentions) = data["mentions"].as_array().filter(|m| !m.is_empty()) {
                mentions.iter().take(80).any(|mention| {
                    is_self(mention) && mention["member_openid"].as_str() == Some(id)
                })
            } else {
                at_event && valid_openid(id) && id != "all"
            }
        })
    } else {
        text.trim()
    }
    .chars()
    .take(16000)
    .collect();
    if text.is_empty() && image_urls.is_empty() && !at_me {
        return Ok(());
    }
    let chat_id = format!("{QQ_OFFICIAL_PREFIX}{kind}_{account}_{recipient}");
    let person = official_person_id(account, sender);
    let author = &data["author"];
    let name: String = ["member_nickname", "username"]
        .into_iter()
        .filter_map(|key| author[key].as_str())
        .find(|name| !name.trim().is_empty())
        .unwrap_or("QQ 用户")
        .chars()
        .take(80)
        .collect();
    let mut c = db::connect(&adapter.0.db_path).await?;
    let ts: Option<i64> = sqlx::query_scalar("SELECT unixepoch(?)")
        .bind(timestamp)
        .fetch_one(&mut *c)
        .await?;
    let Some(ts) = ts.filter(|ts| *ts > 0 && *ts <= yuantuan_core::window::now() + 120) else {
        return Ok(());
    };
    let mut tx = c.begin_immediate().await?;
    let seen: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_messages WHERE platform=? AND account=? AND message_key=?)",
    )
    .bind(QQ_OFFICIAL_ID)
    .bind(account)
    .bind(key)
    .fetch_one(&mut *tx)
    .await?;
    if seen {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES(?1,?2,?3,?3)
         ON CONFLICT(person_id) DO UPDATE SET last_seen=MAX(excluded.last_seen,persons.last_seen)",
    )
    .bind(&person)
    .bind(&name)
    .bind(ts)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT OR IGNORE INTO identities(person_id,platform,platform_uid) VALUES(?,?,?)")
        .bind(&person)
        .bind(QQ_OFFICIAL_ID)
        .bind(format!("{account}:{sender}"))
        .execute(&mut *tx)
        .await?;
    let has_image = !image_urls.is_empty();
    let msg_id: i64 = sqlx::query_scalar(
        "INSERT INTO messages(chat_id,chat_type,sender_pid,nickname,text,mentions,at_me,has_image,ts)
         VALUES(?,?,?,?,?,?,?,?,?) RETURNING msg_id",
    )
    .bind(&chat_id)
    .bind(kind)
    .bind(&person)
    .bind(&name)
    .bind(&text)
    .bind(json!(mentions).to_string())
    .bind(at_me)
    .bind(has_image)
    .bind(ts)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO platform_messages(msg_id,platform,account,message_key,recipient) VALUES(?,?,?,?,?)",
    )
    .bind(msg_id)
    .bind(QQ_OFFICIAL_ID)
    .bind(account)
    .bind(key)
    .bind(recipient)
    .execute(&mut *tx)
    .await?;
    for (index, url) in image_urls.iter().enumerate() {
        sqlx::query("INSERT INTO message_images(msg_id,image_index,url) VALUES(?,?,?)")
            .bind(msg_id)
            .bind(index as i64)
            .bind(url)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    adapter
        .0
        .bus
        .publish(Event::MessageReceived(MessageReceivedPayload {
            msg_id,
            chat_id,
            chat_type: kind.into(),
            sender_pid: person,
            text,
            at_me,
            has_image,
            reply_to: None,
            sender_bot: false,
            image_urls,
            ts,
        }));
    Ok(())
}
