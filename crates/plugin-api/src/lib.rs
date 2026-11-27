//! Stable source-level contract for compiled Rust plugins; not a dynamic-library ABI.
mod account;
pub use account::require_account_access;
mod membership;
mod qzone;
mod schema;
pub use schema::validate as validate_tool_arguments;
mod skills;
mod tools;
pub use membership::*;
pub use qzone::*;
pub use skills::*;
pub use tools::*;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub const API_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize)]
pub struct PluginMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub api_version: u32,
}

/// Optional product copy; capability IDs and persisted settings remain stable.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PluginPresentation {
    pub purpose: &'static str,
    pub poem: &'static str,
    pub verse: &'static str,
    pub examples: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    String,
    Boolean,
    Integer,
    Choice,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConfigOption {
    pub label: &'static str,
    pub value: &'static str,
}

/// A small form schema understood by the host, with optional plugin validation.
#[derive(Clone, Debug, Serialize)]
pub struct ConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ConfigKind,
    pub default: Value,
    pub secret: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ConfigOption>,
}

pub fn validate_config(fields: &[ConfigField], value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("配置必须是对象"))?;
    for (key, value) in object {
        let field = fields
            .iter()
            .find(|f| f.key == key)
            .ok_or_else(|| anyhow::anyhow!("未知配置项：{key}"))?;
        let valid = match field.kind {
            ConfigKind::String => value.is_string(),
            ConfigKind::Boolean => value.is_boolean(),
            ConfigKind::Integer => value.is_i64(),
            ConfigKind::Choice => value
                .as_str()
                .is_some_and(|value| field.options.iter().any(|option| option.value == value)),
        };
        if !valid {
            bail!("配置项 {key} 的类型不正确");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub document_id: String,
    pub title: String,
    pub text: String,
    pub source: String,
}

/// Restricted group actions; the adapter must verify live roles and message ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModerationAction {
    Recall { message_id: i64 },
    Ban { user_id: u64, duration_secs: u32 },
}

#[async_trait]
pub trait GroupModeration: Send + Sync {
    async fn execute(&self, ctx: &ToolCtx, action: ModerationAction) -> Result<()>;
    async fn membership(
        &self,
        _ctx: &ToolCtx,
        _action: MembershipAction,
        _policy: &Value,
        _allowed_chats: Option<&[String]>,
    ) -> Result<Value> {
        bail!("适配器尚未提供申请审核服务")
    }
}

/// Generic protocol backend gateway (e.g. OneBot 11, Telegram, Discord, Feishu).
/// Core protocol backends implement this so platform-specific plugins can invoke
/// protocol actions without coupling core or plugin-api to a single platform.
#[async_trait]
pub trait ProtocolGateway: Send + Sync {
    fn protocol_id(&self) -> &'static str;
    fn account_id(&self) -> Option<String> {
        None
    }
    fn is_connected(&self) -> bool {
        true
    }
    async fn call_action(&self, ctx: &ToolCtx, action: &str, params: Value) -> Result<Value>;
}

/// Extensible capability registry passed from the host/adapters to the plugin manager.
/// Replaces positional platform-specific service arguments.
#[derive(Clone, Default)]
pub struct ServiceRegistry {
    protocols: BTreeMap<String, Arc<dyn ProtocolGateway>>,
    extensions: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_protocol(&mut self, gateway: Arc<dyn ProtocolGateway>) -> &mut Self {
        self.protocols
            .insert(gateway.protocol_id().to_string(), gateway);
        self
    }

    pub fn protocol(&self, id: &str) -> Option<&Arc<dyn ProtocolGateway>> {
        self.protocols.get(id)
    }

    pub fn protocols(&self) -> impl Iterator<Item = (&str, &Arc<dyn ProtocolGateway>)> {
        self.protocols.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn register<T: ?Sized + Send + Sync + 'static>(&mut self, service: Arc<T>) -> &mut Self {
        self.extensions
            .insert(TypeId::of::<Arc<T>>(), Arc::new(service));
        self
    }

    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.extensions
            .get(&TypeId::of::<Arc<T>>())
            .and_then(|v| v.downcast_ref::<Arc<T>>())
            .cloned()
    }
}

/// Host services are scoped to one plugin. Call context is supplied by the host.
/// Native Rust code is trusted code: this interface is not an OS sandbox.
#[async_trait]
pub trait HostServices: Send + Sync {
    /// Configured account administrator; never chosen by model/tool arguments.
    fn account_owner_qq(&self) -> Option<u64> {
        None
    }
    /// Platform-agnostic configured account owner identifier.
    fn account_owner(&self) -> Option<String> {
        self.account_owner_qq().map(|id| id.to_string())
    }
    /// Returns true if the caller in `ctx` matches the configured account owner.
    fn is_account_owner(&self, ctx: &ToolCtx) -> bool {
        let Some(owner) = self.account_owner() else {
            return false;
        };
        let raw = ctx.sender_pid.strip_prefix("p_").unwrap_or(&ctx.sender_pid);
        !raw.is_empty() && raw == owner
    }
    /// Invoke an action on a registered core protocol backend (e.g. "onebot").
    async fn call_protocol(
        &self,
        _ctx: &ToolCtx,
        protocol: &str,
        _action: &str,
        _params: Value,
    ) -> Result<Value> {
        bail!("协议后端 {protocol} 未启用")
    }
    /// Connected account identifier for the given protocol backend, if available.
    fn protocol_account(&self, _protocol: &str) -> Option<String> {
        None
    }
    /// Whether the given protocol backend is currently connected.
    fn is_protocol_connected(&self, _protocol: &str) -> bool {
        false
    }
    async fn qzone(&self, _ctx: &ToolCtx, _action: QzoneAction) -> Result<Value> {
        bail!("QQ空间服务未启用")
    }
    async fn membership(&self, _ctx: &ToolCtx, _action: MembershipAction) -> Result<Value> {
        bail!("宿主未提供申请审核服务")
    }
    async fn moderate(&self, _ctx: &ToolCtx, _action: ModerationAction) -> Result<()> {
        bail!("宿主未提供群管理服务")
    }
    async fn chat(&self, ctx: &ToolCtx, system: &str, user: &str) -> Result<String>;
    /// Shared registered Tool switch, scope and schema check for Skill delegates.
    async fn authorize_tool_for_skill(
        &self,
        _ctx: &ToolCtx,
        _name: &str,
        _args: &Value,
    ) -> Result<()> {
        bail!("宿主未提供 Tool 权限检查")
    }
    async fn read_data(&self, key: &str) -> Result<Option<Value>>;
    async fn write_data(&self, key: &str, value: Value) -> Result<()>;
    fn log(&self, message: &str);
    /// Reserve a host-owned output name. Write the completed file before returning it.
    async fn artifact_path(&self, _extension: &str) -> Result<String> {
        bail!("宿主未提供附件存储")
    }
    async fn search_knowledge(
        &self,
        _ctx: &ToolCtx,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<KnowledgeHit>> {
        bail!("知识库尚未启用")
    }
}

#[derive(Clone)]
pub struct PluginContext {
    pub host: Arc<dyn HostServices>,
}

#[derive(Default)]
pub struct Capabilities {
    pub tools: Vec<Arc<dyn Tool>>,
    pub skills: Vec<Arc<dyn Skill>>,
}

#[derive(Clone, Debug)]
pub struct PluginEvent {
    pub kind: String,
    pub payload: Value,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn metadata(&self) -> PluginMeta;
    fn presentation(&self) -> PluginPresentation {
        PluginPresentation::default()
    }
    /// Opt in to the restricted host service; adapter authorization still applies.
    fn uses_group_moderation(&self) -> bool {
        false
    }
    /// Opt in to host-mediated Qzone access and image encoding.
    fn uses_qzone(&self) -> bool {
        false
    }
    fn default_timeout_ms(&self) -> u64 {
        30_000
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![]
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)
    }
    /// Called at startup or after a configuration reload, following validation and stop.
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities>;
    /// Stop accepting work before this hook; cancel owned tasks here.
    async fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn subscriptions(&self) -> &'static [&'static str] {
        &[]
    }
    async fn on_event(&self, _event: PluginEvent) -> Result<()> {
        Ok(())
    }
}
