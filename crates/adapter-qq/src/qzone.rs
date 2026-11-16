//! Publish through the existing authenticated NapCat connection; no exported cookies.
use crate::AdapterHandle;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

pub fn qzone_service(handle: AdapterHandle, owner: Option<u64>) -> Arc<dyn QzoneService> {
    Arc::new(Service { handle, owner })
}
struct Service {
    handle: AdapterHandle,
    owner: Option<u64>,
}

fn authorize(ctx: &ToolCtx, owner: Option<u64>, bot: u64) -> Result<()> {
    let actor = ctx
        .sender_pid
        .strip_prefix("p_")
        .and_then(|s| s.parse::<u64>().ok());
    ensure!(
        matches!(ctx.chat_type.as_str(), "group" | "private")
            && actor.is_some_and(|a| a > 0 && Some(a) == owner && a != bot),
        ToolError::new(
            ToolErrorKind::PermissionDenied,
            "仅云团超级管理员可以操作QQ空间，请检查security.super_admin_qq"
        )
    );
    Ok(())
}

#[async_trait]
impl QzoneService for Service {
    async fn execute(&self, ctx: &ToolCtx, action: QzoneAction) -> Result<Value> {
        authorize(ctx, self.owner, self.handle.self_qq())?;
        let sender = self
            .handle
            .sender()
            .await
            .ok_or_else(|| ToolError::new(ToolErrorKind::Unavailable, "NapCat未连接"))?;
        ensure!(
            self.handle.self_qq() > 0,
            ToolError::new(ToolErrorKind::Unavailable, "QQ登录尚未就绪")
        );
        match action {
            QzoneAction::CheckAccess { account_qq } => {
                ensure!(
                    account_qq == self.handle.self_qq(),
                    ToolError::new(
                        ToolErrorKind::Unavailable,
                        "QQ账号已改变，请重新确认发布账号"
                    )
                );
                Ok(json!({"qq":account_qq}))
            }
            QzoneAction::Status => {
                let version = sender
                    .action("get_version_info", json!({}))
                    .await
                    .map_err(|_| {
                        ToolError::new(ToolErrorKind::Unavailable, "NapCat状态查询失败")
                    })?;
                Ok(
                    json!({"connected":true,"qq":self.handle.self_qq(),"version":version["app_version"].as_str().unwrap_or("未知").chars().take(80).collect::<String>()}),
                )
            }
            QzoneAction::Publish { post, account_qq } => {
                let login = sender
                    .action("get_login_info", json!({}))
                    .await
                    .map_err(|_| {
                        ToolError::new(ToolErrorKind::Unavailable, "发布前QQ账号核对失败")
                    })?;
                ensure!(
                    login["user_id"].as_u64() == Some(account_qq)
                        && account_qq == self.handle.self_qq(),
                    ToolError::new(
                        ToolErrorKind::Unavailable,
                        "QQ账号已改变，请重新确认发布账号"
                    )
                );
                let params = publish_params(&post)?;
                let raw = sender
                    .action("send_qzone_msg", params)
                    .await
                    .map_err(|_| unknown())?;
                receipt(self.handle.self_qq(), post.visibility, &raw)
            }
        }
    }
}
fn unknown() -> ToolError {
    ToolError::new(
        ToolErrorKind::ResultUnknown,
        "说说发布结果未确认，请先到QQ空间核对，勿自动重发",
    )
}
fn publish_params(post: &QzonePost) -> Result<Value> {
    post.validate()
        .map_err(|_| ToolError::new(ToolErrorKind::InvalidArguments, "说说参数不合规"))?;
    ensure!(
        post.images.iter().all(|s| s.starts_with("base64://"))
            && post.images.iter().map(String::len).sum::<usize>() <= 34 * 1024 * 1024,
        ToolError::new(
            ToolErrorKind::InvalidArguments,
            "图片未经宿主校验或超过大小限制"
        )
    );
    Ok(json!({"content":post.content,"images":post.images,"ugc_right":post.visibility.ugc_right()}))
}
fn receipt(qq: u64, visibility: QzoneVisibility, raw: &Value) -> Result<Value> {
    let tid = raw["tid"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or_else(unknown)?;
    Ok(
        json!({"status":"published","tid":tid,"qq":qq,"visibility":visibility,
        "url":format!("https://user.qzone.qq.com/{qq}/mood/{tid}")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(actor: &str) -> ToolCtx {
        ToolCtx {
            task_id: None,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: actor.into(),
            locale: None,
        }
    }
    #[test]
    fn only_configured_owner_can_publish_not_other_group_administrators_or_the_bot() {
        assert!(authorize(&ctx("p_123"), Some(123), 999).is_ok());
        for (actor, owner, bot) in [
            ("p_124", Some(123), 999),
            ("p_123", None, 999),
            ("p_123", Some(123), 123),
            ("self", Some(123), 999),
        ] {
            assert!(authorize(&ctx(actor), owner, bot).is_err());
        }
    }
    #[test]
    fn native_visibility_and_receipts_match_the_installed_napcat_contract() {
        for (visibility, right) in [
            (QzoneVisibility::Public, 1),
            (QzoneVisibility::Friends, 4),
            (QzoneVisibility::Private, 64),
        ] {
            let post = QzonePost {
                content: "今天做完了插件".into(),
                images: vec![],
                visibility,
            };
            assert_eq!(publish_params(&post).unwrap()["ugc_right"], right);
            assert_eq!(
                receipt(999, visibility, &json!({"tid":"a_b-123"})).unwrap()["status"],
                "published"
            );
        }
        assert!(receipt(999, QzoneVisibility::Friends, &json!({"ok":true})).is_err());
        assert!(receipt(999, QzoneVisibility::Friends, &json!({"tid":"../other"})).is_err());
        let post = QzonePost {
            content: "图片".into(),
            images: vec!["file:///etc/passwd".into()],
            visibility: QzoneVisibility::Friends,
        };
        assert!(publish_params(&post).is_err());
    }
}
