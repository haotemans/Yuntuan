use crate::Settings;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

const ONEBOT: &str = "onebot";

#[derive(Clone, Copy)]
pub(crate) enum Op {
    GroupInfo,
    MemberInfo,
    UserInfo,
    EssenceList,
    Poke,
    GroupNotice,
}

impl Op {
    pub(crate) fn has_skill(self) -> bool {
        matches!(
            self,
            Self::GroupInfo | Self::MemberInfo | Self::EssenceList | Self::Poke
        )
    }

    fn tool_name(self) -> &'static str {
        match self {
            Self::GroupInfo => "qq.group_info",
            Self::MemberInfo => "qq.member_info",
            Self::UserInfo => "qq.user_info",
            Self::EssenceList => "qq.essence_list",
            Self::Poke => "qq.poke",
            Self::GroupNotice => "qq.group_notice",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::GroupInfo => "查询当前 QQ 群的基础信息（群名、成员数等）",
            Self::MemberInfo => "查询当前 QQ 群指定成员的群名片、角色与活跃信息",
            Self::UserInfo => "查询指定 QQ 号用户的公开基础资料",
            Self::EssenceList => "查看当前 QQ 群的精华消息列表",
            Self::Poke => "在当前 QQ 群或私聊中发送一次戳一戳",
            Self::GroupNotice => "由超级管理员在当前 QQ 群发布群公告",
        }
    }

    fn policy(self) -> ToolPolicy {
        match self {
            Self::GroupInfo | Self::MemberInfo | Self::UserInfo | Self::EssenceList => ToolPolicy {
                effect: ToolEffect::ReadOnly,
                timeout_ms: 15_000,
                cancellation: ToolCancellation::DropFuture,
                parallel_safe: true,
            },
            Self::Poke | Self::GroupNotice => ToolPolicy {
                effect: ToolEffect::ExternalWrite,
                timeout_ms: 15_000,
                cancellation: ToolCancellation::AfterCall,
                parallel_safe: false,
            },
        }
    }

    fn schema(self) -> Value {
        match self {
            Self::GroupInfo | Self::EssenceList => json!({
                "type": "object",
                "properties": {
                    "no_cache": { "type": "boolean", "description": "是否跳过缓存" }
                }
            }),
            Self::MemberInfo => json!({
                "type": "object",
                "properties": {
                    "user_id": { "type": "integer", "minimum": 10000, "description": "目标 QQ 号；省略时查询当前发送者" },
                    "no_cache": { "type": "boolean", "description": "是否跳过缓存" }
                }
            }),
            Self::UserInfo => json!({
                "type": "object",
                "required": ["user_id"],
                "properties": {
                    "user_id": { "type": "integer", "minimum": 10000, "description": "目标 QQ 号" },
                    "no_cache": { "type": "boolean", "description": "是否跳过缓存" }
                }
            }),
            Self::Poke => json!({
                "type": "object",
                "properties": {
                    "user_id": { "type": "integer", "minimum": 10000, "description": "目标 QQ 号；私聊可省略，群聊默认戳当前发送者" }
                }
            }),
            Self::GroupNotice => json!({
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": { "type": "string", "minLength": 1, "maxLength": 2000, "description": "群公告正文" }
                }
            }),
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::GroupInfo => "/群信息",
            Self::MemberInfo => "/群成员",
            Self::EssenceList => "/精华消息",
            Self::Poke => "/戳一戳",
            Self::UserInfo | Self::GroupNotice => "",
        }
    }

    fn skill_description(self) -> &'static str {
        match self {
            Self::GroupInfo => "查看当前 QQ 群信息",
            Self::MemberInfo => "查看当前 QQ 群成员信息，可指定 QQ 号",
            Self::EssenceList => "查看当前 QQ 群精华消息",
            Self::Poke => "发送 QQ 戳一戳，可指定 QQ 号",
            Self::UserInfo | Self::GroupNotice => "",
        }
    }
}

pub(crate) struct Handler {
    pub host: Arc<dyn HostServices>,
    pub settings: Settings,
    pub op: Op,
}

fn ensure_onebot(ctx: &ToolCtx) -> Result<()> {
    ensure!(
        ctx.inferred_platform() == ONEBOT,
        ToolError::new(
            ToolErrorKind::Unavailable,
            "当前会话不是 OneBot QQ 通道，无法执行该 QQ 平台操作"
        )
    );
    Ok(())
}

fn parse_numeric_id(raw: &str, label: &str) -> Result<u64> {
    let stripped = raw.strip_prefix("p_").unwrap_or(raw);
    let id = stripped.parse::<u64>().ok().filter(|&n| n > 0).ok_or_else(|| {
        ToolError::new(
            ToolErrorKind::InvalidArguments,
            format!("无法从当前上下文解析有效的{label}"),
        )
    })?;
    Ok(id)
}

fn group_id_from_ctx(ctx: &ToolCtx) -> Result<u64> {
    ensure!(
        ctx.chat_type == "group",
        ToolError::new(ToolErrorKind::PermissionDenied, "该操作仅支持 QQ 群聊")
    );
    parse_numeric_id(&ctx.chat_id, "群号")
}

fn user_id_arg(args: &Value, ctx: &ToolCtx, required: bool) -> Result<u64> {
    if let Some(v) = args.get("user_id") {
        if let Some(n) = v.as_u64().filter(|&n| n >= 10000) {
            return Ok(n);
        }
        if let Some(s) = v.as_str() {
            if let Ok(n) = s.trim().parse::<u64>() {
                if n >= 10000 {
                    return Ok(n);
                }
            }
        }
        return Err(ToolError::new(
            ToolErrorKind::InvalidArguments,
            "user_id 必须是有效的 QQ 号（>= 10000）",
        )
        .into());
    }
    ensure!(
        !required,
        ToolError::new(ToolErrorKind::InvalidArguments, "缺少 user_id 参数")
    );
    parse_numeric_id(&ctx.sender_pid, "QQ 号")
}

impl Handler {
    async fn execute(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        ensure_onebot(ctx)?;
        match self.op {
            Op::GroupInfo => {
                ensure!(
                    self.settings.allow_group_queries,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭群信息查询")
                );
                let group_id = group_id_from_ctx(ctx)?;
                let no_cache = args["no_cache"].as_bool().unwrap_or(false);
                let data = self
                    .host
                    .call_protocol(
                        ctx,
                        ONEBOT,
                        "get_group_info",
                        json!({ "group_id": group_id, "no_cache": no_cache }),
                    )
                    .await?;
                let name = data["group_name"].as_str().unwrap_or("未知群名");
                let count = data["member_count"].as_u64().unwrap_or(0);
                let max = data["max_member_count"].as_u64().unwrap_or(0);
                Ok(ToolOutput {
                    summary: format!("群 {name}（{group_id}），当前成员 {count}/{max}"),
                    artifacts: vec![],
                    data,
                })
            }
            Op::MemberInfo => {
                ensure!(
                    self.settings.allow_group_queries,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭群成员查询")
                );
                let group_id = group_id_from_ctx(ctx)?;
                let user_id = user_id_arg(&args, ctx, false)?;
                let no_cache = args["no_cache"].as_bool().unwrap_or(false);
                let data = self
                    .host
                    .call_protocol(
                        ctx,
                        ONEBOT,
                        "get_group_member_info",
                        json!({
                            "group_id": group_id,
                            "user_id": user_id,
                            "no_cache": no_cache,
                        }),
                    )
                    .await?;
                let card = data["card"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| data["nickname"].as_str())
                    .unwrap_or("未知成员");
                let role = data["role"].as_str().unwrap_or("member");
                Ok(ToolOutput {
                    summary: format!("群成员 {card}（{user_id}），角色：{role}"),
                    artifacts: vec![],
                    data,
                })
            }
            Op::UserInfo => {
                ensure!(
                    self.settings.allow_group_queries,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭用户资料查询")
                );
                let user_id = user_id_arg(&args, ctx, true)?;
                let no_cache = args["no_cache"].as_bool().unwrap_or(false);
                let data = self
                    .host
                    .call_protocol(
                        ctx,
                        ONEBOT,
                        "get_stranger_info",
                        json!({ "user_id": user_id, "no_cache": no_cache }),
                    )
                    .await?;
                let nick = data["nickname"].as_str().unwrap_or("未知用户");
                Ok(ToolOutput {
                    summary: format!("QQ 用户 {nick}（{user_id}）"),
                    artifacts: vec![],
                    data,
                })
            }
            Op::EssenceList => {
                ensure!(
                    self.settings.allow_group_queries,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭精华消息查询")
                );
                let group_id = group_id_from_ctx(ctx)?;
                let data = self
                    .host
                    .call_protocol(
                        ctx,
                        ONEBOT,
                        "get_essence_msg_list",
                        json!({ "group_id": group_id }),
                    )
                    .await?;
                let count = data.as_array().map_or(0, |a| a.len());
                Ok(ToolOutput {
                    summary: format!("本群共有 {count} 条精华消息"),
                    artifacts: vec![],
                    data,
                })
            }
            Op::Poke => {
                ensure!(
                    self.settings.allow_poke,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭戳一戳功能")
                );
                let user_id = user_id_arg(&args, ctx, false)?;
                let params = if ctx.chat_type == "group" {
                    let group_id = group_id_from_ctx(ctx)?;
                    json!({ "group_id": group_id, "user_id": user_id })
                } else {
                    json!({ "user_id": user_id })
                };
                let data = self
                    .host
                    .call_protocol(ctx, ONEBOT, "send_poke", params)
                    .await?;
                Ok(ToolOutput {
                    summary: format!("已向 {user_id} 发送戳一戳"),
                    artifacts: vec![],
                    data,
                })
            }
            Op::GroupNotice => {
                ensure!(
                    self.settings.allow_owner_notice,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件未启用群公告发布")
                );
                ensure!(
                    self.host.is_account_owner(ctx),
                    ToolError::new(
                        ToolErrorKind::PermissionDenied,
                        "仅云团超级管理员可发布群公告"
                    )
                );
                let group_id = group_id_from_ctx(ctx)?;
                let content = args["content"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.chars().count() <= 2000)
                    .ok_or_else(|| {
                        ToolError::new(
                            ToolErrorKind::InvalidArguments,
                            "群公告正文须为 1–2000 字",
                        )
                    })?;
                let data = self
                    .host
                    .call_protocol(
                        ctx,
                        ONEBOT,
                        "_send_group_notice",
                        json!({ "group_id": group_id, "content": content }),
                    )
                    .await?;
                Ok(ToolOutput {
                    summary: format!("已在群 {group_id} 发布群公告"),
                    artifacts: vec![],
                    data,
                })
            }
        }
    }
}

#[async_trait]
impl Tool for Handler {
    fn name(&self) -> &'static str {
        self.op.tool_name()
    }

    fn description(&self) -> &'static str {
        self.op.description()
    }

    fn available_in_chat(&self, ctx: &ToolCtx) -> bool {
        if ctx.inferred_platform() != ONEBOT {
            return false;
        }
        match self.op {
            Op::GroupInfo | Op::MemberInfo | Op::EssenceList => {
                self.settings.allow_group_queries && ctx.chat_type == "group"
            }
            Op::UserInfo => self.settings.allow_group_queries,
            Op::Poke => self.settings.allow_poke,
            Op::GroupNotice => {
                self.settings.allow_owner_notice
                    && ctx.chat_type == "group"
                    && self.host.is_account_owner(ctx)
            }
        }
    }

    fn policy(&self) -> ToolPolicy {
        self.op.policy()
    }

    fn parameters_schema(&self) -> Value {
        self.op.schema()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        validate_tool_arguments(&self.parameters_schema(), &args)?;
        self.execute(ctx, args).await
    }
}

#[async_trait]
impl Skill for Handler {
    fn def(&self) -> SkillDef {
        SkillDef {
            name: self.op.tool_name(),
            description: self.op.skill_description(),
            input_schema: self.op.schema(),
            output_type: SkillOutputType::Text,
        }
    }

    fn accepts_empty_command(&self) -> bool {
        true
    }

    fn match_message(&self, msg: &IncomingMessage) -> Option<Value> {
        let cmd = self.op.command();
        if cmd.is_empty() {
            return None;
        }
        let text = msg.text.trim();
        if text == cmd {
            return Some(json!({}));
        }
        let rest = text.strip_prefix(cmd)?.trim();
        if rest.is_empty() {
            return Some(json!({}));
        }
        if matches!(self.op, Op::MemberInfo | Op::Poke) {
            if let Ok(uid) = rest.parse::<u64>() {
                return Some(json!({ "user_id": uid }));
            }
        }
        None
    }

    async fn execute(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let args = if slots.is_object() { slots } else { json!({}) };
        self.host
            .authorize_tool_for_skill(ctx, self.op.tool_name(), &args)
            .await?;
        self.execute(ctx, args).await
    }
}
