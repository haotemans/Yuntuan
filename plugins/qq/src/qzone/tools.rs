use super::publisher::Publisher;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

#[derive(Clone, Copy)]
pub(super) enum Op {
    Publish,
    Status,
    History,
}
pub(super) struct Handler {
    pub publisher: Arc<Publisher>,
    pub op: Op,
}
impl Handler {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Publish => "qzone.publish",
            Op::Status => "qzone.status",
            Op::History => "qzone.history",
        }
    }
    fn description(&self) -> &'static str {
        match self.op {
        Op::Publish=>"仅超级管理员明确要求时发布QQ空间说说，支持正文、云团已有图片和可见范围；回执不确定时不可重试。",
        Op::Status=>"查询当前QQ空间发布连接状态，仅超级管理员可用。",
        Op::History=>"查看本账号最近10条说说发布回执和待核实记录，仅超级管理员可用。"}
    }
    fn args(&self, mut args: Value) -> Value {
        if matches!(self.op, Op::Publish) && args.get("visibility").is_none() {
            args["visibility"] = json!(self.publisher.visibility);
        }
        args
    }
    async fn execute(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        match self.op {
            Op::Publish => self.publisher.publish(ctx, self.args(args)).await,
            Op::Status => self.publisher.status(ctx).await,
            Op::History => self.publisher.history(ctx).await,
        }
    }
}
#[async_trait]
impl Tool for Handler {
    fn name(&self) -> &'static str {
        self.name()
    }
    fn description(&self) -> &'static str {
        self.description()
    }
    fn policy(&self) -> ToolPolicy {
        ToolPolicy {
            effect: if matches!(self.op, Op::Publish) {
                ToolEffect::ExternalWrite
            } else {
                ToolEffect::ReadOnly
            },
            timeout_ms: 120_000,
            cancellation: ToolCancellation::AfterCall,
            parallel_safe: false,
        }
    }
    fn parameters_schema(&self) -> Value {
        if !matches!(self.op, Op::Publish) {
            return json!({"type":"object","properties":{},"additionalProperties":false});
        }
        json!({"type":"object","properties":{"content":{"type":"string","maxLength":2000,"description":"用户明确要求发布的正文；纯图片时为空字符串"},
          "images":{"type":"array","maxItems":9,"items":{"type":"string"},"description":"云团图库或插件artifacts中已存在的本地图片路径；不接受URL或任意系统文件"},
          "visibility":{"type":"string","enum":["public","friends","private"],"description":"可见范围，省略沿用插件默认配置"}},"required":["content"],"additionalProperties":false})
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        self.execute(ctx, args).await
    }
}
#[async_trait]
impl Skill for Handler {
    fn def(&self) -> SkillDef {
        SkillDef {
            name: self.name(),
            description: self.description(),
            prompt_template: "".into(),
            tools: vec![self.name()],
            role_hint: Some(include_str!("SKILL.md")),
        }
    }
    fn accepts_empty_command(&self) -> bool {
        !matches!(self.op, Op::Publish)
    }
    fn commands(&self) -> &'static [&'static str] {
        match self.op {
            Op::Publish => &["/说说", "/发说说"],
            Op::Status => &["/空间状态"],
            Op::History => &["/空间记录"],
        }
    }
    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let args = if matches!(self.op, Op::Publish) {
            if slots["explicit_command"] == true {
                let text = slots["user_text"].as_str().unwrap_or("").trim();
                let mut content = text;
                let mut visibility = self.publisher.visibility;
                for (flag, value) in [
                    ("--公开", QzoneVisibility::Public),
                    ("--好友", QzoneVisibility::Friends),
                    ("--私密", QzoneVisibility::Private),
                ] {
                    if let Some(rest) = text
                        .strip_prefix(flag)
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        content = rest.trim_start();
                        visibility = value;
                        break;
                    }
                }
                json!({"content":content,"visibility":visibility})
            } else {
                let mut args = json!({"content":slots["content"]});
                for key in ["images", "visibility"] {
                    if let Some(v) = slots.get(key) {
                        args[key] = v.clone();
                    }
                }
                self.args(args)
            }
        } else {
            json!({})
        };
        // Validate the registered Tool switch/scope/schema without recursively taking its gate.
        self.publisher
            .host
            .authorize_tool_for_skill(ctx, self.name(), &args)
            .await?;
        self.execute(ctx, args).await
    }
}
