//! Unified QQ platform plugin consolidating QQ protocol actions, group moderation/membership, and QQ Space.
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use yuantuan_plugin_api::*;

pub mod moderator;
pub mod qzone;
#[cfg(test)]
mod tests;
mod tools;

pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(QqPlatformPlugin::default())
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

#[derive(Default)]
struct QqPlatformPlugin {
    moderator_worker: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for QqPlatformPlugin {
    fn drop(&mut self) {
        if let Ok(worker) = self.moderator_worker.get_mut() {
            if let Some(worker) = worker.take() {
                worker.abort();
            }
        }
    }
}

#[async_trait]
impl Plugin for QqPlatformPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "qq",
            name: "鸿雁",
            version: env!("CARGO_PKG_VERSION"),
            description: "QQ 平台专属能力与协议兼容（群聊交互、撤回禁言审核、QQ空间与协议动作）",
            api_version: API_VERSION,
        }
    }

    fn presentation(&self) -> PluginPresentation {
        PluginPresentation {
            purpose: "QQ 平台全套能力（交互、群管审核、QQ空间）",
            poem: "小雅·鸿雁",
            verse: "鸿雁于飞，肃肃其羽。",
            examples: vec![
                "/群信息",
                "/戳一戳 123456",
                "/撤回",
                "/禁言 @成员 10",
                "/说说 今天完成了一个小目标",
            ],
        }
    }

    fn uses_group_moderation(&self) -> bool {
        true
    }

    fn uses_qzone(&self) -> bool {
        true
    }

    fn default_timeout_ms(&self) -> u64 {
        120_000
    }

    fn config_fields(&self) -> Vec<ConfigField> {
        let mut fields = vec![
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
        ];
        fields.extend(qzone::qzone_config_fields());
        fields.extend(moderator::moderator_config_fields());
        fields
    }

    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)?;
        qzone::validate_qzone_config(config)?;
        moderator::validate_moderator_config(config)?;
        Ok(())
    }

    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        self.validate_config(&config)?;
        let settings = Settings::from_config(&config);
        let mut caps = Capabilities::default();

        for op in [
            tools::Op::ChannelStatus,
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

        qzone::register_qzone_capabilities(&ctx, &config, &mut caps)?;
        let worker = moderator::register_moderator_capabilities(&ctx, &config, &mut caps)?;
        if let Some(previous) = self.moderator_worker.lock().unwrap().replace(worker) {
            previous.abort();
        }
        Ok(caps)
    }

    async fn stop(&self) -> Result<()> {
        if let Some(worker) = self.moderator_worker.lock().unwrap().take() {
            worker.abort();
        }
        Ok(())
    }
}
