//! Narrow QQ Space service; the adapter owns account authorization and OneBot transport.
use crate::ToolCtx;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QzoneVisibility {
    Public,
    #[default]
    Friends,
    Private,
}
impl QzoneVisibility {
    pub fn ugc_right(self) -> u8 {
        match self {
            Self::Public => 1,
            Self::Friends => 4,
            Self::Private => 64,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QzonePost {
    pub content: String,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub visibility: QzoneVisibility,
}
impl QzonePost {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.content.trim().is_empty() || !self.images.is_empty(),
            "说说需要正文或图片"
        );
        ensure!(
            self.content.chars().count() <= 2000 && !self.content.contains('\0'),
            "说说正文最多2000字且不能含NUL"
        );
        ensure!(self.images.len() <= 9, "说说最多9张图片");
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum QzoneAction {
    Status,
    CheckAccess { account_qq: u64 },
    Publish { post: QzonePost, account_qq: u64 },
}

#[async_trait]
pub trait QzoneService: Send + Sync {
    /// Publish images are host-validated, bounded base64 payloads, never arbitrary URLs/paths.
    async fn execute(&self, ctx: &ToolCtx, action: QzoneAction) -> Result<Value>;
}
