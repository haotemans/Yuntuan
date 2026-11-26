//! Compiled plugin host with runtime configuration reload; native code is trusted.
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use yuantuan_plugin_api::*;
mod invocation;
mod qzone;
use invocation::Invocations;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginSettings {
    pub enabled: bool,
    /// None permits all chats; an empty list permits none.
    pub allowed_chats: Option<Vec<String>>,
    pub timeout_ms: u64,
    pub max_concurrency: usize,
    pub max_output_bytes: usize,
    pub config: Value,
}
impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_chats: None,
            timeout_ms: 30_000,
            max_concurrency: 1,
            max_output_bytes: 65_536,
            config: json!({}),
        }
    }
}
impl PluginSettings {
    fn validate(&self) -> Result<()> {
        if !(100..=360_000).contains(&self.timeout_ms)
            || !(1..=16).contains(&self.max_concurrency)
            || !(1024..=1_048_576).contains(&self.max_output_bytes)
        {
            bail!("插件运行限制超出范围");
        }
        if self
            .allowed_chats
            .as_ref()
            .is_some_and(|c| c.len() > 1000 || c.iter().any(|s| s.is_empty() || s.len() > 128))
        {
            bail!("群范围配置无效");
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

struct Gate {
    active: AtomicBool,
    policy: PluginSettings,
    permits: Arc<Semaphore>,
    artifact_root: PathBuf,
    invocations: Arc<Invocations>,
    secrets: Vec<String>,
}
impl Gate {
    async fn invoke(
        &self,
        chat: &str,
        future: impl std::future::Future<Output = Result<ToolOutput>>,
    ) -> Result<ToolOutput> {
        let _permit = match self.enter(Some(chat)).await {
            Ok(permit) => permit,
            Err(error) => {
                self.invocations.reject();
                return Err(error);
            }
        };
        let call = self.invocations.begin();
        let result = self
            .run(future)
            .await
            .and_then(|out| self.check_output(out))
            .map_err(|error| {
                let message = redact_error(&error.to_string(), &self.secrets);
                if let Some(typed) = error.downcast_ref::<ToolError>() {
                    ToolError::new(typed.kind, message).into()
                } else {
                    anyhow::anyhow!(message)
                }
            });
        call.finish(result.as_ref().err().map(ToString::to_string));
        result
    }
    async fn enter(&self, chat: Option<&str>) -> Result<tokio::sync::OwnedSemaphorePermit> {
        if !self.active.load(Ordering::Acquire) {
            return Err(ToolError::new(ToolErrorKind::Unavailable, "插件已停止").into());
        }
        if let Some(allowed) = &self.policy.allowed_chats {
            if !chat.is_some_and(|id| allowed.iter().any(|c| c == id)) {
                return Err(ToolError::new(
                    ToolErrorKind::PermissionDenied,
                    "当前会话未启用此插件",
                )
                .into());
            }
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| ToolError::new(ToolErrorKind::Unavailable, "插件忙，请稍后重试"))?;
        if !self.active.load(Ordering::Acquire) {
            return Err(ToolError::new(ToolErrorKind::Unavailable, "插件已停止").into());
        }
        Ok(permit)
    }
    async fn run<T>(&self, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(
            Duration::from_millis(self.policy.timeout_ms),
            std::panic::AssertUnwindSafe(future).catch_unwind(),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ToolError::new(ToolErrorKind::Panic, "插件调用发生 panic").into()),
            Err(_) => Err(ToolError::new(ToolErrorKind::Timeout, "插件调用超时").into()),
        }
    }
    fn check_output(&self, out: ToolOutput) -> Result<ToolOutput> {
        if !out.artifacts.is_empty() {
            if out.artifacts.len() > 30 {
                bail!("附件数量超过 30");
            }
            let root = self
                .artifact_root
                .canonicalize()
                .context("插件附件目录不存在")?;
            for path in &out.artifacts {
                let file = PathBuf::from(path)
                    .canonicalize()
                    .context("插件附件不存在")?;
                if !file.starts_with(&root) || !file.is_file() {
                    bail!("附件不在本插件的输出目录中");
                }
                if !matches!(
                    file.extension().and_then(|s| s.to_str()),
                    Some("mp4" | "jpg" | "png" | "webp" | "gif")
                ) {
                    bail!("不支持的附件格式");
                }
            }
        }

        let bytes = out.summary.len()
            + out.artifacts.iter().map(String::len).sum::<usize>()
            + serde_json::to_vec(&out.data)?.len();
        if bytes > self.policy.max_output_bytes {
            bail!("插件返回结果过大");
        }
        Ok(out)
    }
}
struct HostedTool {
    inner: Arc<dyn Tool>,
    gate: Arc<Gate>,
}
#[async_trait]
impl Tool for HostedTool {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn description(&self) -> &'static str {
        self.inner.description()
    }
    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }
    fn available_in_chat(&self, ctx: &ToolCtx) -> bool {
        self.gate.active.load(Ordering::Acquire)
            && self
                .gate
                .policy
                .allowed_chats
                .as_ref()
                .is_none_or(|a| a.contains(&ctx.chat_id))
            && self.inner.available_in_chat(ctx)
    }
    fn policy(&self) -> yuantuan_plugin_api::ToolPolicy {
        self.inner.policy()
    }
    fn output_schema(&self) -> Option<Value> {
        self.inner.output_schema()
    }
    fn audit_details(&self, args: &Value, output: &ToolOutput) -> Option<Value> {
        self.inner.audit_details(args, output)
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        self.inner.artifact_retention()
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        let result = self
            .gate
            .invoke(&ctx.chat_id, self.inner.call(ctx, args))
            .await;
        tracing::info!(
            tool = self.name(),
            success = result.is_ok(),
            "插件工具调用完成"
        );
        result
    }
}
struct HostedSkill {
    inner: Arc<dyn Skill>,
    gate: Arc<Gate>,
}
#[async_trait]
impl Skill for HostedSkill {
    fn is_information_lookup(&self) -> bool {
        self.inner.is_information_lookup()
    }
    fn available_in_chat(&self, chat_id: &str) -> bool {
        self.gate.active.load(Ordering::Acquire)
            && self
                .gate
                .policy
                .allowed_chats
                .as_ref()
                .is_none_or(|allowed| allowed.iter().any(|chat| chat == chat_id))
            && self.inner.available_in_chat(chat_id)
    }
    fn match_message(&self, ctx: &ToolCtx, text: &str) -> Option<Value> {
        if !self.gate.active.load(Ordering::Acquire)
            || self
                .gate
                .policy
                .allowed_chats
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(&ctx.chat_id))
        {
            return None;
        }
        self.inner.match_message(ctx, text)
    }
    fn delivery_policy(&self) -> DeliveryPolicy {
        self.inner.delivery_policy()
    }
    fn def(&self) -> SkillDef {
        self.inner.def()
    }
    fn commands(&self) -> &'static [&'static str] {
        self.inner.commands()
    }
    fn accepts_empty_command(&self) -> bool {
        self.inner.accepts_empty_command()
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        self.inner.artifact_retention()
    }
    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let result = self
            .gate
            .invoke(&ctx.chat_id, self.inner.invoke(ctx, slots))
            .await;
        tracing::info!(
            skill = self.def().name,
            success = result.is_ok(),
            "插件技能调用完成"
        );
        result
    }
}

struct Services {
    id: &'static str,
    directory: PathBuf,
    llm: crate::bot::SharedLlm,
    policy: PluginSettings,
    io: Mutex<()>,
    services: ServiceRegistry,
    uses_group_moderation: bool,
    tools: crate::tools::Registry,
    db_path: PathBuf,
    account_owner: Arc<AtomicU64>,
    owner_identities: Arc<RwLock<Vec<String>>>,
}
#[async_trait]
impl HostServices for Services {
    fn account_owner_qq(&self) -> Option<u64> {
        let id = self.account_owner.load(Ordering::Acquire);
        (id > 0).then_some(id)
    }
    fn account_owner(&self) -> Option<String> {
        self.owner_identities
            .read()
            .unwrap()
            .first()
            .cloned()
            .or_else(|| self.account_owner_qq().map(|id| id.to_string()))
    }
    fn is_account_owner(&self, ctx: &ToolCtx) -> bool {
        let raw = ctx.sender_pid.strip_prefix("p_").unwrap_or(&ctx.sender_pid);
        if raw.is_empty() {
            return false;
        }
        let owners = self.owner_identities.read().unwrap();
        if owners.iter().any(|o| o == raw || o == &ctx.sender_pid) {
            return true;
        }
        self.account_owner_qq()
            .is_some_and(|qq| raw == qq.to_string())
    }
    async fn call_protocol(
        &self,
        ctx: &ToolCtx,
        protocol: &str,
        action: &str,
        params: Value,
    ) -> Result<Value> {
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            return Err(ToolError::new(
                ToolErrorKind::PermissionDenied,
                "会话不在插件范围内",
            )
            .into());
        }
        let gateway = self.services.protocol(protocol).ok_or_else(|| {
            ToolError::new(
                ToolErrorKind::Unavailable,
                format!("协议后端 {protocol} 未启用"),
            )
        })?;
        gateway.call_action(ctx, action, params).await
    }
    fn protocol_account(&self, protocol: &str) -> Option<String> {
        self.services.protocol(protocol).and_then(|g| g.account_id())
    }
    fn is_protocol_connected(&self, protocol: &str) -> bool {
        self.services
            .protocol(protocol)
            .is_some_and(|g| g.is_connected())
    }
    async fn authorize_tool_for_skill(
        &self,
        ctx: &ToolCtx,
        name: &str,
        args: &Value,
    ) -> Result<()> {
        if !name.starts_with(&format!("{}.", self.id)) {
            bail!("Skill 不得调用其他插件的工具");
        }
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            bail!("会话不在插件授权范围内");
        }
        let tool = self.tools.get(name).context("Tool 未注册或已经被停用")?;
        if !tool.available_in_chat(ctx) {
            bail!("Tool 在当前会话不可用");
        }
        crate::capabilities::check_tool(&self.db_path, name, ctx, &tool.parameters_schema(), args)
            .await
    }
    async fn qzone(&self, ctx: &ToolCtx, action: QzoneAction) -> Result<Value> {
        qzone::execute(self, ctx, action).await
    }
    async fn membership(&self, ctx: &ToolCtx, action: MembershipAction) -> Result<Value> {
        if (self.id != "moderator" && self.id != "qq") || !self.uses_group_moderation {
            bail!("此插件不能管理好友或入群申请");
        }
        self.services
            .get::<dyn GroupModeration>()
            .context("QQ申请审核服务未启用")?
            .membership(
                ctx,
                action,
                &self.policy.config,
                self.policy.allowed_chats.as_deref(),
            )
            .await
    }
    async fn moderate(&self, ctx: &ToolCtx, action: ModerationAction) -> Result<()> {
        if !self.uses_group_moderation {
            bail!("插件未声明群管理能力");
        }
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            bail!("会话不在插件范围内");
        }
        self.services
            .get::<dyn GroupModeration>()
            .context("插件未声明群管理能力或 QQ 适配器未启用")?
            .execute(ctx, action)
            .await
    }
    async fn chat(&self, ctx: &ToolCtx, system: &str, user: &str) -> Result<String> {
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            bail!("会话不在插件范围内");
        }
        let gateway = self.llm.read().unwrap().clone().context("尚未配置模型")?;
        tokio::time::timeout(
            Duration::from_millis(self.policy.timeout_ms),
            gateway.chat(crate::llm::Role::BotChat, system, user, false),
        )
        .await
        .context("模型调用超时")?
    }
    async fn read_data(&self, key: &str) -> Result<Option<Value>> {
        if !valid_id(key) {
            bail!("数据键只能包含小写字母、数字、下划线和连字符");
        }
        let _guard = self.io.lock().await;
        let path = self.directory.join(format!("kv-{key}.json"));
        match tokio::fs::metadata(&path).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(meta) if meta.len() > 1_048_576 => bail!("插件数据超过 1 MiB"),
            Err(e) => return Err(e.into()),
            _ => {}
        }
        Ok(Some(serde_json::from_slice(&tokio::fs::read(path).await?)?))
    }
    async fn write_data(&self, key: &str, value: Value) -> Result<()> {
        if !valid_id(key) {
            bail!("数据键无效");
        }
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > 1_048_576 {
            bail!("插件数据超过 1 MiB");
        }
        let _guard = self.io.lock().await;
        atomic_write(&self.directory.join(format!("kv-{key}.json")), &bytes).await
    }
    async fn artifact_path(&self, extension: &str) -> Result<String> {
        if !["mp4", "jpg", "png", "webp", "gif"].contains(&extension) {
            bail!("不支持的附件格式");
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = self.directory.join("artifacts");
        tokio::fs::create_dir_all(&dir).await?;
        let dir = tokio::fs::canonicalize(dir).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        Ok(dir
            .join(format!(
                "{now}-{}.{}",
                NEXT.fetch_add(1, Ordering::Relaxed),
                extension
            ))
            .to_string_lossy()
            .into_owned())
    }
    fn log(&self, message: &str) {
        tracing::info!(plugin = self.id, message = %message.chars().take(2048).collect::<String>(), "插件日志");
    }
}
async fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    tokio::fs::create_dir_all(path.parent().context("缺少父目录")?).await?;
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(&temporary, path).await?;
    Ok(())
}

fn secret_values(plugin: &dyn Plugin, settings: &PluginSettings) -> Vec<String> {
    let mut values: Vec<_> = plugin
        .config_fields()
        .iter()
        .filter(|field| field.secret)
        .filter_map(|field| {
            settings.config[field.key]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .collect();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values
}

fn redact_error(message: &str, secrets: &[String]) -> String {
    let mut message = message.to_owned();
    for secret in secrets {
        message = message.replace(secret, "[已隐藏]");
    }
    message.chars().take(600).collect()
}

struct Entry {
    revision: AtomicU64,
    plugin: Arc<dyn Plugin>,
    meta: PluginMeta,
    runtime: RwLock<Runtime>,
    desired: RwLock<PluginSettings>,
    error: RwLock<Option<String>>,
    directory: PathBuf,
    restart_required: AtomicBool,
    updating: AtomicBool,
    invocations: Arc<Invocations>,
}
#[derive(Clone)]
struct Runtime {
    applied: PluginSettings,
    gate: Arc<Gate>,
    tools: Vec<&'static str>,
    skills: Vec<SkillDef>,
}
pub struct PluginManager {
    entries: BTreeMap<&'static str, Entry>,
    tools: crate::tools::Registry,
    skills: crate::skills::SkillRegistry,
    events: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    lifecycle: tokio::sync::RwLock<()>,
    stopped: AtomicBool,
    llm: crate::bot::SharedLlm,
    services: ServiceRegistry,
    db_path: PathBuf,
    account_owner: Arc<AtomicU64>,
    owner_identities: Arc<RwLock<Vec<String>>>,
}

enum Change {
    Reload,
    Settings(PluginSettings),
    Toggle(bool),
    ConfigPatch(serde_json::Map<String, Value>, Option<bool>),
}
struct UpdateGuard {
    manager: Arc<PluginManager>,
    id: String,
}
impl Drop for UpdateGuard {
    fn drop(&mut self) {
        self.manager.entries[self.id.as_str()]
            .updating
            .store(false, Ordering::Release);
    }
}

async fn update_response(handle: tokio::task::JoinHandle<Result<()>>) -> Result<bool> {
    match tokio::time::timeout(Duration::from_millis(200), handle).await {
        Ok(result) => {
            result??;
            Ok(false)
        }
        Err(_) => Ok(true), // Dropping a JoinHandle detaches; the accepted change continues.
    }
}
impl PluginManager {
    pub fn set_account_owner(&self, owner: Option<u64>) {
        self.account_owner
            .store(owner.unwrap_or(0), Ordering::Release);
        let mut owners = self.owner_identities.write().unwrap();
        if let Some(id) = owner.filter(|&n| n > 0) {
            let s = id.to_string();
            if !owners.contains(&s) {
                owners.insert(0, s);
            }
        }
    }
    /// Register platform-agnostic owner identifiers (e.g. OpenIDs, non-numeric user IDs).
    pub fn set_owner_identities(&self, ids: Vec<String>) {
        let mut owners = self.owner_identities.write().unwrap();
        for id in ids {
            if !id.trim().is_empty() && !owners.contains(&id) {
                owners.push(id);
            }
        }
    }
    pub async fn load(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
    ) -> Result<Arc<Self>> {
        Self::load_with_moderation(plugins, data_root, source_root, tools, skills, llm, None).await
    }
    pub async fn load_with_moderation(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
        moderation: Option<Arc<dyn GroupModeration>>,
    ) -> Result<Arc<Self>> {
        Self::load_with_services(
            plugins,
            data_root,
            source_root,
            tools,
            skills,
            llm,
            moderation,
            None,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn load_with_services(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
        moderation: Option<Arc<dyn GroupModeration>>,
        qzone: Option<Arc<dyn QzoneService>>,
    ) -> Result<Arc<Self>> {
        let mut services = ServiceRegistry::new();
        if let Some(m) = moderation {
            services.register::<dyn GroupModeration>(m);
        }
        if let Some(q) = qzone {
            services.register::<dyn QzoneService>(q);
        }
        Self::load_with_registry(plugins, data_root, source_root, tools, skills, llm, services)
            .await
    }
    pub async fn load_with_registry(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
        services: ServiceRegistry,
    ) -> Result<Arc<Self>> {
        let mut manager = Self {
            entries: BTreeMap::new(),
            tools,
            skills,
            events: Mutex::new(vec![]),
            lifecycle: tokio::sync::RwLock::new(()),
            stopped: AtomicBool::new(false),
            llm: llm.clone(),
            services,
            db_path: data_root.join("yuantuan.db"),
            account_owner: Arc::new(AtomicU64::new(0)),
            owner_identities: Arc::new(RwLock::new(vec![])),
        };
        // Validate the complete catalog before starting any plugin.
        let mut ids = std::collections::HashSet::new();
        for plugin in &plugins {
            let meta = plugin.metadata();
            if !valid_id(meta.id) || !ids.insert(meta.id) {
                bail!("插件 ID 无效或重复：{}", meta.id);
            }
        }
        for plugin in plugins {
            let meta = plugin.metadata();
            let directory = data_root.join("plugins").join(meta.id);
            let mut settings = PluginSettings {
                timeout_ms: plugin.default_timeout_ms(),
                enabled: source_root.join(meta.id).join("enabled").exists(),
                ..Default::default()
            };
            let mut error = None;
            match tokio::fs::read(directory.join("settings.json")).await {
                Ok(bytes) => match serde_json::from_slice::<PluginSettings>(&bytes) {
                    Ok(saved) => settings = saved,
                    Err(_) => error = Some("插件配置文件损坏".into()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => error = Some("插件配置文件不可读".into()),
            }
            if let Some(config) = settings.config.as_object_mut() {
                for field in plugin.config_fields() {
                    config.entry(field.key).or_insert(field.default);
                }
            }
            if meta.api_version != API_VERSION {
                error = Some("插件接口版本不兼容".into());
            }
            if error.is_none() {
                if let Err(e) = settings
                    .validate()
                    .and_then(|_| plugin.validate_config(&settings.config))
                {
                    error = Some(e.to_string());
                }
            }
            let invocations = Arc::new(Invocations::default());
            let gate = Arc::new(Gate {
                active: AtomicBool::new(false),
                permits: Arc::new(Semaphore::new(settings.max_concurrency.min(16))),
                policy: settings.clone(),
                artifact_root: directory.join("artifacts"),
                invocations: invocations.clone(),
                secrets: secret_values(plugin.as_ref(), &settings),
            });
            let mut tool_names = vec![];
            let mut skill_defs = vec![];
            if settings.enabled && error.is_none() {
                let host = Arc::new(Services {
                    id: meta.id,
                    directory: directory.clone(),
                    llm: llm.clone(),
                    policy: settings.clone(),
                    io: Mutex::new(()),
                    tools: manager.tools.clone(),
                    db_path: manager.db_path.clone(),
                    account_owner: manager.account_owner.clone(),
                    owner_identities: manager.owner_identities.clone(),
                    services: manager.services.clone(),
                    uses_group_moderation: plugin.uses_group_moderation(),
                });
                let result = gate
                    .run(plugin.start(PluginContext { host }, settings.config.clone()))
                    .await;
                let result = result.and_then(|caps| {
                    let prefix = format!("{}.", meta.id);
                    if caps.tools.iter().any(|t| !t.name().starts_with(&prefix))
                        || caps
                            .skills
                            .iter()
                            .any(|s| !s.def().name.starts_with(&prefix))
                    {
                        bail!("插件能力名必须以 {prefix} 开头");
                    }
                    tool_names = caps.tools.iter().map(|t| t.name()).collect();
                    skill_defs = caps.skills.iter().map(|s| s.def()).collect();
                    manager.tools.register_batch(
                        caps.tools
                            .into_iter()
                            .map(|inner| {
                                Arc::new(HostedTool {
                                    inner,
                                    gate: gate.clone(),
                                }) as Arc<dyn Tool>
                            })
                            .collect(),
                    )?;
                    if let Err(e) = manager.skills.register_batch(
                        caps.skills
                            .into_iter()
                            .map(|inner| {
                                Arc::new(HostedSkill {
                                    inner,
                                    gate: gate.clone(),
                                }) as Arc<dyn Skill>
                            })
                            .collect(),
                    ) {
                        manager.tools.unregister(&tool_names);
                        return Err(e);
                    }
                    Ok(())
                });
                match result {
                    Ok(()) => {
                        gate.active.store(true, Ordering::Release);
                    }
                    Err(e) => {
                        error = Some(e.to_string());
                        tool_names.clear();
                        skill_defs.clear();
                        let _ = gate.run(plugin.stop()).await;
                    }
                }
            }
            manager.entries.insert(
                meta.id,
                Entry {
                    revision: AtomicU64::new(rand::random::<u64>()),
                    plugin,
                    meta,
                    desired: RwLock::new(settings.clone()),
                    runtime: RwLock::new(Runtime {
                        applied: settings,
                        gate,
                        tools: tool_names,
                        skills: skill_defs,
                    }),
                    error: RwLock::new(error),
                    directory,
                    restart_required: AtomicBool::new(false),
                    updating: AtomicBool::new(false),
                    invocations,
                },
            );
        }
        Ok(Arc::new(manager))
    }
    pub fn list(&self) -> Vec<Value> {
        self.entries.values().map(|e| {
            let desired = e.desired.read().unwrap();
            let runtime = e.runtime.read().unwrap();
            let loaded = runtime.gate.active.load(Ordering::Acquire);
            let fields = e.plugin.config_fields();
            let secrets = secret_values(e.plugin.as_ref(), &desired);
            let mut config = desired.config.clone();
            for field in &fields {
                if field.secret { config.as_object_mut().map(|c| c.remove(field.key)); }
            }
            let mut settings = serde_json::to_value(&*desired).unwrap();
            settings["config"] = config;
            let skills = runtime.skills.iter().map(|s| {
                let commands = self.skills.get(s.name).map(|skill| skill.commands().to_vec()).unwrap_or_default();
                json!({"name":s.name, "description":s.description, "commands":commands})
            }).collect::<Vec<_>>();
            json!({
                "name":e.meta.id, "display_name":e.meta.name, "version":e.meta.version,
                "revision":e.revision.load(Ordering::Acquire).to_string(),
                "description":e.meta.description, "presentation":e.plugin.presentation(),
                "api_version":e.meta.api_version, "enabled":desired.enabled, "loaded":loaded,
                "requires_onebot":e.plugin.uses_group_moderation() || e.meta.id == "qzone" || e.meta.id == "qq",
                "requires_group":e.plugin.uses_group_moderation(),
                "updating":e.updating.load(Ordering::Acquire),
                "pending_restart":e.restart_required.load(Ordering::Acquire),
                "pending_reload":*desired != runtime.applied || (desired.enabled && !loaded),
                "code_update_requires_restart":true,
                "error":e.error.read().unwrap().as_ref().map(|error| redact_error(error, &secrets)),
                "data_dir":e.directory.to_string_lossy(),
                "tools":runtime.tools.iter().map(|n| json!({"name":n})).collect::<Vec<_>>(),
                "skills":skills, "invocations":e.invocations.snapshot(),
                "config_fields":fields.iter().map(|field| {
                    let mut value = serde_json::to_value(field).unwrap();
                    if field.secret {
                        value["default"] = Value::Null;
                        value["has_value"] = json!(desired.config[field.key].as_str().is_some_and(|value| !value.is_empty()));
                    }
                    value
                }).collect::<Vec<_>>(),
                "settings":settings,
            })
        }).collect()
    }

    fn start_change(
        self: &Arc<Self>,
        id: &str,
        change: Change,
        expected: Option<u64>,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        anyhow::ensure!(!self.stopped.load(Ordering::Acquire), "插件宿主已停止");
        let entry = self.entries.get(id).context("插件未编译进当前程序")?;
        anyhow::ensure!(
            !entry.restart_required.load(Ordering::Acquire),
            "插件未能安全停止，需要重启服务后再加载"
        );
        anyhow::ensure!(
            entry
                .updating
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "插件正在应用配置，请稍候"
        );
        let guard = UpdateGuard {
            manager: self.clone(),
            id: id.to_owned(),
        };
        let old = entry.desired.read().unwrap().clone();
        anyhow::ensure!(expected.is_none_or(|v| v == entry.revision.load(Ordering::Acquire)), "插件状态已变化，请重新查询后再修改");
        let force_reload = matches!(change, Change::Reload);
        let mut settings = match change {
            Change::Reload => old.clone(),
            Change::Settings(settings) => settings,
            Change::Toggle(enabled) => PluginSettings {
                enabled,
                ..old.clone()
            },
            Change::ConfigPatch(patch, enabled) => {
                let mut settings = old.clone();
                let config = settings
                    .config
                    .as_object_mut()
                    .context("插件配置必须是对象")?;
                for (key, value) in patch {
                    anyhow::ensure!(
                        entry.plugin.config_fields().iter().any(|f| f.key == key),
                        "未知插件配置字段"
                    );
                    config.insert(key, value);
                }
                if let Some(enabled) = enabled {
                    settings.enabled = enabled;
                }
                settings
            }
        };
        if let Some(config) = settings.config.as_object_mut() {
            for field in entry.plugin.config_fields() {
                config
                    .entry(field.key)
                    .or_insert_with(|| old.config.get(field.key).cloned().unwrap_or(field.default));
            }
        }
        let mut secrets = secret_values(entry.plugin.as_ref(), &old);
        secrets.extend(secret_values(entry.plugin.as_ref(), &settings));
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        secrets.dedup();
        settings
            .validate()
            .and_then(|_| entry.plugin.validate_config(&settings.config))
            .map_err(|error| anyhow::anyhow!(redact_error(&error.to_string(), &secrets)))?;
        anyhow::ensure!(
            serde_json::to_vec(&settings)?.len() <= 65_536,
            "插件配置超过 64 KiB"
        );
        entry.revision.fetch_add(1, Ordering::AcqRel);
        Ok(tokio::spawn(async move {
            let _lifecycle = guard.manager.lifecycle.read().await;
            anyhow::ensure!(
                !guard.manager.stopped.load(Ordering::Acquire),
                "插件宿主已停止"
            );
            let entry = &guard.manager.entries[guard.id.as_str()];
            let result = guard.manager.save_locked(entry, settings, force_reload).await;
            if let Err(error) = &result {
                *entry.error.write().unwrap() = Some(redact_error(&error.to_string(), &secrets));
            }
            result.map_err(|error| anyhow::anyhow!(redact_error(&error.to_string(), &secrets)))
        }))
    }

    pub async fn save(self: &Arc<Self>, id: &str, settings: PluginSettings) -> Result<()> {
        self.start_change(id, Change::Settings(settings), None)?.await?
    }
    /// Apply onboarding results under the existing update guard, retaining secrets,
    /// scopes and limits rather than rebuilding settings from the redacted catalog.
    pub async fn patch_config(
        self: &Arc<Self>,
        id: &str,
        patch: serde_json::Map<String, Value>,
        enabled: Option<bool>,
    ) -> Result<()> {
        self.start_change(id, Change::ConfigPatch(patch, enabled), None)?
            .await?
    }
    /// Return promptly for long drains. The catalog exposes progress and the final error.
    pub async fn save_deferred(
        self: &Arc<Self>,
        id: &str,
        settings: PluginSettings,
    ) -> Result<bool> {
        update_response(self.start_change(id, Change::Settings(settings), None)?).await
    }
    pub async fn toggle_deferred(self: &Arc<Self>, id: &str, enabled: bool) -> Result<bool> {
        update_response(self.start_change(id, Change::Toggle(enabled), None)?).await
    }
    pub async fn managed_change(self: &Arc<Self>, id: &str, revision: u64, patch: serde_json::Map<String,Value>, enabled: Option<bool>, reload: bool) -> Result<bool> {
        let change = if reload { Change::Reload } else { Change::ConfigPatch(patch,enabled) };
        update_response(self.start_change(id, change, Some(revision))?).await
    }
    async fn save_locked(&self, entry: &Entry, mut settings: PluginSettings, force_reload: bool) -> Result<()> {
        anyhow::ensure!(
            !entry.restart_required.load(Ordering::Acquire),
            "插件未能安全停止，需要重启服务后再加载"
        );
        let old = entry.desired.read().unwrap().clone();
        // Omitted secret fields retain their old values; empty strings explicitly clear them.
        if let Some(config) = settings.config.as_object_mut() {
            for field in entry.plugin.config_fields() {
                if !config.contains_key(field.key) {
                    config.insert(
                        field.key.into(),
                        old.config.get(field.key).cloned().unwrap_or(field.default),
                    );
                }
            }
        }
        settings.validate()?;
        entry.plugin.validate_config(&settings.config)?;
        let bytes = serde_json::to_vec_pretty(&settings)?;
        if bytes.len() > 65_536 {
            bail!("插件配置超过 64 KiB");
        }
        atomic_write(&entry.directory.join("settings.json"), &bytes).await?;
        *entry.desired.write().unwrap() = settings.clone();
        let old = entry.runtime.read().unwrap().clone();
        if !force_reload && old.applied == settings && (old.gate.active.load(Ordering::Acquire) || !settings.enabled)
        {
            *entry.error.write().unwrap() = None;
            return Ok(());
        }
        if let Err(error) = self.deactivate(entry, &old).await {
            *entry.error.write().unwrap() = Some(error.to_string());
            return Err(error);
        }
        match self.activate(entry, settings).await {
            Ok(runtime) => {
                *entry.runtime.write().unwrap() = runtime;
                *entry.error.write().unwrap() = None;
                Ok(())
            }
            Err(error) => {
                if entry.restart_required.load(Ordering::Acquire) {
                    let note = format!("热重载失败，插件清理未完成，需要重启服务：{error}");
                    *entry.error.write().unwrap() = Some(note.clone());
                    bail!(note);
                }
                // Rebuild the previous instance so a bad update does not take down other plugins.
                let restored = self.activate(entry, old.applied.clone()).await;
                let note = match restored {
                    Ok(runtime) => {
                        *entry.runtime.write().unwrap() = runtime;
                        format!("配置已保存，热重载失败，已恢复原配置：{error}")
                    }
                    Err(restore_error) => {
                        format!("配置已保存，热重载失败：{error}；恢复失败：{restore_error}")
                    }
                };
                *entry.error.write().unwrap() = Some(note.clone());
                bail!(note)
            }
        }
    }
    pub async fn toggle(self: &Arc<Self>, id: &str, enabled: bool) -> Result<()> {
        self.start_change(id, Change::Toggle(enabled), None)?.await?
    }
    pub async fn subscribe(self: &Arc<Self>, bus: &crate::event::EventBus) {
        let _guard = self.lifecycle.write().await;
        let mut handles = self.events.lock().await;
        if !handles.is_empty() || self.stopped.load(Ordering::Acquire) {
            return;
        }
        for (id, entry) in &self.entries {
            if entry.plugin.subscriptions().is_empty() {
                continue;
            }
            let mut receiver = bus.subscribe();
            let manager = self.clone();
            let id = *id;
            handles.push(tokio::spawn(async move {
                let entry = &manager.entries[id];
                loop {
                    let event = match receiver.recv().await {
                        Ok(e) => e,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(plugin = id, skipped = n, "插件事件消费滞后");
                            continue;
                        }
                        Err(_) => break,
                    };
                    if !entry.plugin.subscriptions().contains(&event.kind()) {
                        continue;
                    }
                    let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
                    let gate = entry.runtime.read().unwrap().gate.clone();
                    let Ok(_permit) = gate
                        .enter(payload.get("chat_id").and_then(Value::as_str))
                        .await
                    else {
                        continue;
                    };
                    let result = gate
                        .run(entry.plugin.on_event(PluginEvent {
                            kind: event.kind().into(),
                            payload,
                        }))
                        .await;
                    if result.is_err() {
                        tracing::warn!(plugin = id, "插件事件处理失败");
                    }
                }
            }));
        }
    }
    async fn deactivate(&self, entry: &Entry, runtime: &Runtime) -> Result<()> {
        let was_active = runtime.gate.active.swap(false, Ordering::AcqRel);
        // No new invocations can enter. Let calls and event handlers finish before stop/start.
        let _drain = runtime
            .gate
            .permits
            .clone()
            .acquire_many_owned(runtime.applied.max_concurrency.min(16) as u32)
            .await?;
        if was_active {
            if let Err(error) = runtime.gate.run(entry.plugin.stop()).await {
                entry.restart_required.store(true, Ordering::Release);
                self.tools.unregister(&runtime.tools);
                self.skills
                    .unregister(&runtime.skills.iter().map(|s| s.name).collect::<Vec<_>>());
                return Err(error.context("插件停止失败，未启动新实例"));
            }
        }
        self.tools.unregister(&runtime.tools);
        self.skills
            .unregister(&runtime.skills.iter().map(|s| s.name).collect::<Vec<_>>());
        Ok(())
    }

    async fn activate(&self, entry: &Entry, settings: PluginSettings) -> Result<Runtime> {
        anyhow::ensure!(entry.meta.api_version == API_VERSION, "插件接口版本不兼容");
        let gate = Arc::new(Gate {
            active: AtomicBool::new(false),
            permits: Arc::new(Semaphore::new(settings.max_concurrency)),
            policy: settings.clone(),
            artifact_root: entry.directory.join("artifacts"),
            invocations: entry.invocations.clone(),
            secrets: secret_values(entry.plugin.as_ref(), &settings),
        });
        let mut runtime = Runtime {
            applied: settings.clone(),
            gate: gate.clone(),
            tools: vec![],
            skills: vec![],
        };
        if !settings.enabled {
            return Ok(runtime);
        }
        let host = Arc::new(Services {
            id: entry.meta.id,
            directory: entry.directory.clone(),
            llm: self.llm.clone(),
            policy: settings.clone(),
            io: Mutex::new(()),
            tools: self.tools.clone(),
            db_path: self.db_path.clone(),
            account_owner: self.account_owner.clone(),
            owner_identities: self.owner_identities.clone(),
            services: self.services.clone(),
            uses_group_moderation: entry.plugin.uses_group_moderation(),
        });
        let result = async {
            let caps = gate
                .run(entry.plugin.start(PluginContext { host }, settings.config))
                .await?;
            let prefix = format!("{}.", entry.meta.id);
            anyhow::ensure!(
                caps.tools.iter().all(|t| t.name().starts_with(&prefix))
                    && caps
                        .skills
                        .iter()
                        .all(|s| s.def().name.starts_with(&prefix)),
                "插件能力名必须以 {prefix} 开头"
            );
            let tools = caps.tools.iter().map(|t| t.name()).collect::<Vec<_>>();
            let skills = caps.skills.iter().map(|s| s.def()).collect::<Vec<_>>();
            self.tools.register_batch(
                caps.tools
                    .into_iter()
                    .map(|inner| {
                        Arc::new(HostedTool {
                            inner,
                            gate: gate.clone(),
                        }) as Arc<dyn Tool>
                    })
                    .collect(),
            )?;
            if let Err(error) = self.skills.register_batch(
                caps.skills
                    .into_iter()
                    .map(|inner| {
                        Arc::new(HostedSkill {
                            inner,
                            gate: gate.clone(),
                        }) as Arc<dyn Skill>
                    })
                    .collect(),
            ) {
                self.tools.unregister(&tools);
                return Err(error);
            }
            runtime.tools = tools;
            runtime.skills = skills;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if let Err(cleanup_error) = gate.run(entry.plugin.stop()).await {
                entry.restart_required.store(true, Ordering::Release);
                return Err(cleanup_error.context("插件启动失败后的清理失败"));
            }
            return Err(error);
        }
        gate.active.store(true, Ordering::Release);
        Ok(runtime)
    }

    pub async fn shutdown(&self) {
        let _guard = self.lifecycle.write().await;
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        for handle in self.events.lock().await.drain(..) {
            handle.abort();
            let _ = handle.await;
        }
        for entry in self.entries.values() {
            let runtime = entry.runtime.read().unwrap().clone();
            if let Err(error) = self.deactivate(entry, &runtime).await {
                tracing::warn!(plugin = entry.meta.id, %error, "插件停止失败");
            }
        }
    }
}
