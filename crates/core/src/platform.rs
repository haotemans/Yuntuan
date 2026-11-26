use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::db::SqliteExt;

pub const ONEBOT_ID: &str = "onebot";
pub const QQ_OFFICIAL_ID: &str = "qq_official";
pub const QQ_OFFICIAL_PREFIX: &str = "qq-official_";

pub fn chat_platform(id: &str) -> Option<&'static str> {
    id.starts_with(QQ_OFFICIAL_PREFIX).then_some(QQ_OFFICIAL_ID)
}

pub fn valid_openid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub fn official_person_id(app_id: &str, openid: &str) -> String {
    format!("{QQ_OFFICIAL_PREFIX}user_{app_id}_{openid}")
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OfficialQqConfig {
    pub enabled: bool,
    pub app_id: String,
    pub app_secret: String,
    /// Explicit operator-confirmed OpenID to OneBot QQ number mapping for memory reads only.
    pub memory_links: BTreeMap<String, String>,
}

impl std::fmt::Debug for OfficialQqConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfficialQqConfig")
            .field("enabled", &self.enabled)
            .field("app_id", &self.app_id)
            .finish_non_exhaustive()
    }
}

impl OfficialQqConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (!self.enabled || !self.app_id.is_empty())
                && self.app_id.len() <= 32
                && self.app_id.bytes().all(|b| b.is_ascii_digit()),
            "QQ 官方机器人 AppID 应为数字"
        );
        ensure!(
            (!self.enabled || !self.app_secret.is_empty())
                && self.app_secret.len() <= 256
                && self.app_secret.bytes().all(|b| b.is_ascii_graphic())
                && self.app_secret != "***",
            "请填写有效的 QQ 官方机器人 AppSecret"
        );
        ensure!(
            self.memory_links.len() <= 256
                && (self.memory_links.is_empty() || !self.app_id.is_empty())
                && self.memory_links.iter().all(|(openid, qq)| {
                    valid_openid(openid)
                        && qq.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == *qq)
                }),
            "QQ 记忆关联需要 AppID，最多 256 项，格式为 OpenID 对应 QQ 号字符串"
        );
        Ok(())
    }

    pub async fn sync_memory_links(&self, conn: &mut sqlx::SqliteConnection) -> Result<()> {
        let links: Vec<(String, String)> = if self.validate().is_ok() {
            self.memory_links
                .iter()
                .map(|(openid, qq)| (official_person_id(&self.app_id, openid), format!("p_{qq}")))
                .collect()
        } else {
            vec![]
        };
        crate::memory::sync_platform_identity_links(conn, "qq", &links).await
    }
}

#[derive(Clone, Serialize)]
pub struct PlatformStatus {
    pub id: &'static str,
    pub state: &'static str,
    pub account: Option<String>,
    pub detail: Option<String>,
}
