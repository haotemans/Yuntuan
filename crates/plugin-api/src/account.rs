//! Account-bound connector authorization shared with the configured Yuntuan administrator.
use crate::{ToolCtx, ToolError, ToolErrorKind};
use anyhow::{ensure, Result};

pub fn require_account_access(ctx: &ToolCtx, owner: Option<u64>, read_only: bool) -> Result<()> {
    let system_read = read_only
        && ctx.task_id.is_none()
        && ctx.chat_type == "private"
        && ((ctx.chat_id == "plugin-check" && ctx.sender_pid == "webui")
            || (ctx.chat_id == "scheduled" && ctx.sender_pid == "scheduler"));
    let actor = ctx
        .sender_pid
        .strip_prefix("p_")
        .and_then(|s| s.parse::<u64>().ok());
    ensure!(
        system_read
            || (matches!(ctx.chat_type.as_str(), "group" | "private")
                && actor.is_some_and(|id| id > 0 && Some(id) == owner)),
        ToolError::new(
            ToolErrorKind::PermissionDenied,
            "此服务操作仅限 security.super_admin_qq 指定的管理员；请检查账号及会话授权"
        )
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(actor: &str, chat: &str) -> ToolCtx {
        ToolCtx {
            task_id: None,
            chat_type: "private".into(),
            sender_pid: actor.into(),
            chat_id: chat.into(),
            locale: None,
        }
    }
    #[test]
    fn account_owner_and_host_read_contexts_cannot_grant_system_write_access() {
        assert!(require_account_access(&ctx("p_123", "42"), Some(123), false).is_ok());
        for owner in [None, Some(456)] {
            assert!(require_account_access(&ctx("p_123", "42"), owner, true).is_err());
        }
        assert!(require_account_access(&ctx("p_0", "42"), Some(0), true).is_err());
        for (actor, chat) in [("webui", "plugin-check"), ("scheduler", "scheduled")] {
            assert!(require_account_access(&ctx(actor, chat), None, true).is_ok());
            assert!(require_account_access(&ctx(actor, chat), None, false).is_err());
            let mut forged = ctx(actor, chat);
            forged.task_id = Some("untrusted-task".into());
            assert!(require_account_access(&forged, None, true).is_err());
        }
        assert!(require_account_access(&ctx("webui", "42"), None, true).is_err());
    }
}
