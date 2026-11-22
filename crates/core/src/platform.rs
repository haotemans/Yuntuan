use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const ONEBOT_ID: &str = "onebot";
pub const QQ_OFFICIAL_ID: &str = "qq_official";
pub const QQ_OFFICIAL_PREFIX: &str = "qq-official_";

pub fn chat_platform(id: &str) -> Option<&'static str> {
    id.starts_with(QQ_OFFICIAL_PREFIX).then_some(QQ_OFFICIAL_ID)
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OfficialQqConfig {
    pub enabled: bool,
    pub app_id: String,
    pub app_secret: String,
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
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct PlatformStatus {
    pub id: &'static str,
    pub state: &'static str,
    pub account: Option<String>,
    pub detail: Option<String>,
}
