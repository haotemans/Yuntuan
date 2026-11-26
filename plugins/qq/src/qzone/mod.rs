//! QQ Space capability package: commands, Skills and Tools share the same publisher.
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;
mod publisher;
#[cfg(test)]
mod tests;
mod tools;

pub(crate) fn qzone_config_fields() -> Vec<ConfigField> {
    Qzone.config_fields()
}

pub(crate) fn validate_qzone_config(config: &Value) -> Result<()> {
    ensure!(
        config
            .get("daily_limit")
            .is_none_or(|v| v.as_u64().is_some_and(|n| (1..=100).contains(&n))),
        "每日发布上限须为1–100"
    );
    Ok(())
}

pub(crate) fn register_qzone_capabilities(
    ctx: &PluginContext,
    config: &Value,
    caps: &mut Capabilities,
) -> Result<()> {
    let publisher = Arc::new(publisher::Publisher::new(
        ctx.host.clone(),
        serde_json::from_value(
            config
                .get("default_visibility")
                .cloned()
                .unwrap_or(json!("friends")),
        )?,
        config["daily_limit"].as_u64().unwrap_or(10) as usize,
    ));
    for op in [tools::Op::Publish, tools::Op::Status, tools::Op::History] {
        let handler = Arc::new(tools::Handler {
            publisher: publisher.clone(),
            op,
        });
        caps.tools.push(handler.clone());
        caps.skills.push(handler);
    }
    Ok(())
}

pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(Qzone)
}
struct Qzone;
#[async_trait]
impl Plugin for Qzone {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "qzone",
            name: "鹿鸣",
            version: env!("CARGO_PKG_VERSION"),
            api_version: API_VERSION,
            description: "发布文字和图片，选择可见范围，查看发布记录。",
        }
    }
    fn presentation(&self) -> PluginPresentation {
        PluginPresentation {
            purpose: "QQ空间动态发布",
            poem: "小雅·鹿鸣",
            verse: "呦呦鹿鸣，食野之苹。",
            examples: vec![
                "/说说 今天完成了一个小目标",
                "/说说 --私密 记录一下今天",
                "/空间状态",
                "/空间记录",
            ],
        }
    }
    fn default_timeout_ms(&self) -> u64 {
        120_000
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "default_visibility",
                label: "默认可见范围",
                kind: ConfigKind::Choice,
                default: json!("friends"),
                secret: false,
                options: vec![
                    ConfigOption {
                        label: "好友可见",
                        value: "friends",
                    },
                    ConfigOption {
                        label: "所有人可见",
                        value: "public",
                    },
                    ConfigOption {
                        label: "仅自己可见",
                        value: "private",
                    },
                ],
            },
            ConfigField {
                key: "daily_limit",
                label: "每日发布上限（北京时间，包含结果待核实的尝试）",
                kind: ConfigKind::Integer,
                default: json!(10),
                secret: false,
                options: vec![],
            },
        ]
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)?;
        ensure!(
            config
                .get("daily_limit")
                .is_none_or(|v| v.as_u64().is_some_and(|n| (1..=100).contains(&n))),
            "每日发布上限须为1–100"
        );
        Ok(())
    }
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        self.validate_config(&config)?;
        let publisher = Arc::new(publisher::Publisher::new(
            ctx.host,
            serde_json::from_value(
                config
                    .get("default_visibility")
                    .cloned()
                    .unwrap_or(json!("friends")),
            )?,
            config["daily_limit"].as_u64().unwrap_or(10) as usize,
        ));
        let mut caps = Capabilities::default();
        for op in [tools::Op::Publish, tools::Op::Status, tools::Op::History] {
            let handler = Arc::new(tools::Handler {
                publisher: publisher.clone(),
                op,
            });
            caps.tools.push(handler.clone());
            caps.skills.push(handler);
        }
        Ok(caps)
    }
}
