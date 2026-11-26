use crate::Settings;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

const ONEBOT: &str = "onebot";
const QQ_OFFICIAL: &str = "qq_official";

#[derive(Clone, Copy)]
pub(crate) enum Op {
    ChannelStatus,
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
            Self::ChannelStatus
                | Self::GroupInfo
                | Self::MemberInfo
                | Self::EssenceList
                | Self::Poke
        )
    }

    fn tool_name(self) -> &'static str {
        match self {
            Self::ChannelStatus => "qq.channel_status",
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
            Self::ChannelStatus => "查询当前 QQ 通道（OneBot 或 QQ 官方机器人）的连接与账号状态",
            Self::GroupInfo => "查询当前 QQ 群的基础信息（OneBot 返回群详情，官方通道返回通道状态）",
            Self::MemberInfo => "查询当前 QQ 群指定成员的群名片、角色与活跃信息（仅 OneBot）",
            Self::UserInfo => "查询指定 QQ 号用户的公开基础资料（仅 OneBot）",
            Self::EssenceList => "查看当前 QQ 群的精华消息列表（仅 OneBot）",
            Self::Poke => "在当前 QQ 群或私聊中发送一次戳一戳（仅 OneBot）",
            Self::GroupNotice => "由超级管理员在当前 QQ 群发布群公告（仅 OneBot）",
        }
    }

    fn policy(self) -> ToolPolicy {
        match self {
            Self::ChannelStatus
            | Self::GroupInfo
            | Self::MemberInfo
            | Self::UserInfo
            | Self::EssenceList => ToolPolicy {
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
            Self::ChannelStatus => json!({
                "type": "object",
                "properties": {}
            }),
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

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::ChannelStatus => &["/QQ状态", "/频道状态"],
            Self::GroupInfo => &["/群信息"],
            Self::MemberInfo => &["/群成员"],
            Self::EssenceList => &["/精华消息"],
            Self::Poke => &["/戳一戳"],
            Self::UserInfo | Self::GroupNotice => &[],
        }
    }

    fn skill_description(self) -> &'static str {
        match self {
            Self::ChannelStatus => "查看当前 QQ 通道（OneBot 或官方机器人）连接状态",
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
            "当前会话为 QQ 官方机器人通道（qq_official），QQ 官方开放平台暂不支持该功能（仅 OneBot 通道可用）"
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
        let platform = ctx.inferred_platform();
        match self.op {
            Op::ChannelStatus => {
                if platform == QQ_OFFICIAL {
                    let data = self
                        .host
                        .call_protocol(ctx, QQ_OFFICIAL, "get_status", json!({}))
                        .await?;
                    let app_id = data["app_id"].as_str().unwrap_or("未配置");
                    let state = data["state"].as_str().unwrap_or("unknown");
                    return Ok(ToolOutput {
                        summary: format!("当前通道：QQ 官方机器人（AppID: {app_id}，状态：{state}）"),
                        artifacts: vec![],
                        data,
                    });
                }
                let connected = self.host.is_protocol_connected(ONEBOT);
                let account = self
                    .host
                    .protocol_account(ONEBOT)
                    .unwrap_or_else(|| "未知".into());
                let data = self
                    .host
                    .call_protocol(ctx, ONEBOT, "get_login_info", json!({}))
                    .await
                    .unwrap_or_else(|_| {
                        json!({
                            "platform": ONEBOT,
                            "account": account,
                            "connected": connected,
                        })
                    });
                Ok(ToolOutput {
                    summary: format!(
                        "当前通道：OneBot QQ（账号：{account}，连接：{}）",
                        if connected { "在线" } else { "未连接" }
                    ),
                    artifacts: vec![],
                    data,
                })
            }
            Op::GroupInfo => {
                ensure!(
                    self.settings.allow_group_queries,
                    ToolError::new(ToolErrorKind::PermissionDenied, "插件已关闭群信息查询")
                );
                ensure!(
                    ctx.chat_type == "group",
                    ToolError::new(ToolErrorKind::PermissionDenied, "该操作仅支持 QQ 群聊")
                );
                if platform == QQ_OFFICIAL {
                    let status = self
                        .host
                        .call_protocol(ctx, QQ_OFFICIAL, "get_status", json!({}))
                        .await
                        .unwrap_or_else(|_| json!({ "platform": QQ_OFFICIAL }));
                    return Ok(ToolOutput {
                        summary: format!(
                            "当前为 QQ 官方群聊通道（{}），QQ 官方开放平台暂不提供群名称与成员数查询",
                            ctx.chat_id
                        ),
                        artifacts: vec![],
                        data: json!({
                            "platform": QQ_OFFICIAL,
                            "chat_id": ctx.chat_id,
                            "channel": status,
                        }),
                    });
                }
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
                ensure_onebot(ctx)?;
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
                ensure_onebot(ctx)?;
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
                ensure_onebot(ctx)?;
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
                ensure_onebot(ctx)?;
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
                ensure_onebot(ctx)?;
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
        let platform = ctx.inferred_platform();
        match self.op {
            Op::ChannelStatus => matches!(platform, ONEBOT | QQ_OFFICIAL),
            Op::GroupInfo => {
                self.settings.allow_group_queries
                    && ctx.chat_type == "group"
                    && matches!(platform, ONEBOT | QQ_OFFICIAL)
            }
            Op::MemberInfo | Op::EssenceList => {
                platform == ONEBOT
                    && self.settings.allow_group_queries
                    && ctx.chat_type == "group"
            }
            Op::UserInfo => platform == ONEBOT && self.settings.allow_group_queries,
            Op::Poke => platform == ONEBOT && self.settings.allow_poke,
            Op::GroupNotice => {
                platform == ONEBOT
                    && self.settings.allow_owner_notice
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
            prompt_template: String::new(),
            tools: vec![self.op.tool_name()],
            role_hint: None,
        }
    }

    fn is_information_lookup(&self) -> bool {
        matches!(self.op, Op::GroupInfo | Op::MemberInfo | Op::EssenceList)
    }

    fn commands(&self) -> &'static [&'static str] {
        self.op.commands()
    }

    fn accepts_empty_command(&self) -> bool {
        true
    }

    fn match_message(&self, _ctx: &ToolCtx, text: &str) -> Option<Value> {
        let cmd = self.op.commands().first().copied()?;
        let trimmed = text.trim();
        if trimmed == cmd {
            return Some(json!({}));
        }
        let rest = trimmed.strip_prefix(cmd)?.trim();
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

    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let mut args = if slots.is_object() {
            slots.clone()
        } else {
            json!({})
        };
        if matches!(self.op, Op::MemberInfo | Op::Poke) && args.get("user_id").is_none() {
            if let Some(uid) = slot_text(&slots, &["user_text", "user_id"])
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|&n| n >= 10000)
            {
                args["user_id"] = json!(uid);
            }
        }
        self.host
            .authorize_tool_for_skill(ctx, self.op.tool_name(), &args)
            .await?;
        self.execute(ctx, args).await
    }
}
