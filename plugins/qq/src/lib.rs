//! QQ platform capability and compatibility plugin built on top of `HostServices::call_protocol("onebot", ...)`.
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

#[cfg(test)]
mod tests;
mod tools;

pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(QqPlatformPlugin)
}

#[derive(Clone, Copy)]
pub(crate) struct Settings {
    pub allow_poke: bool,
    pub allow_group_queries: bool,
    pub allow_owner_notice: bool,
}

impl Settings {
    fn from_config(config: &Value) -> Self {
        Self {
            allow_poke: config["allow_poke"].as_bool().unwrap_or(true),
            allow_group_queries: config["allow_group_queries"].as_bool().unwrap_or(true),
            allow_owner_notice: config["allow_owner_notice"].as_bool().unwrap_or(false),
        }
    }
}

struct QqPlatformPlugin;

#[async_trait]
impl Plugin for QqPlatformPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "qq",
            name: "鸿雁",
            version: env!("CARGO_PKG_VERSION"),
            description: "QQ 平台专属能力与协议兼容（戳一戳、群/成员信息、精华消息与群公告）",
            api_version: API_VERSION,
        }
    }

    fn presentation(&self) -> PluginPresentation {
        PluginPresentation {
            purpose: "QQ 平台兼容与交互扩展",
            poem: "小雅·鸿雁",
            verse: "鸿雁于飞，肃肃其羽。",
            examples: vec![
                "/群信息",
                "/群成员 123456",
                "/戳一戳 123456",
                "/精华消息",
            ],
        }
    }

    fn default_timeout_ms(&self) -> u64 {
        30_000
    }

    fn config_fields(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "allow_poke",
                label: "允许使用戳一戳（send_poke）",
                kind: ConfigKind::Boolean,
                default: json!(true),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "allow_group_queries",
                label: "允许查询群、成员与精华消息",
                kind: ConfigKind::Boolean,
                default: json!(true),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "allow_owner_notice",
                label: "允许超级管理员发送群公告（_send_group_notice）",
                kind: ConfigKind::Boolean,
                default: json!(false),
                secret: false,
                options: vec![],
            },
        ]
    }

    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        self.validate_config(&config)?;
        let settings = Settings::from_config(&config);
        let mut caps = Capabilities::default();

        for op in [
            tools::Op::GroupInfo,
            tools::Op::MemberInfo,
            tools::Op::UserInfo,
            tools::Op::EssenceList,
            tools::Op::Poke,
            tools::Op::GroupNotice,
        ] {
            let handler = Arc::new(tools::Handler {
                host: ctx.host.clone(),
                settings,
                op,
            });
            caps.tools.push(handler.clone());
            if op.has_skill() {
                caps.skills.push(handler);
            }
        }
        Ok(caps)
    }
}
