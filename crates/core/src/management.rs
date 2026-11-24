//! Typed self-management boundary shared by authenticated WebUI and chat tools.
use crate::{
    db,
    plugins::PluginManager,
    tools::{Tool, ToolCtx, ToolEffect, ToolOutput, ToolPolicy},
};
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{Connection, Row};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock, Weak,
    },
};

const POLICY_KEY: &str = "management.grants.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub qq: String,
    pub plugins: Vec<String>,
    pub configure: bool,
    pub toggle: bool,
    pub reload: bool,
}
#[derive(Clone)]
pub enum Caller {
    Webui,
    Chat(ToolCtx),
}
impl Caller {
    fn actor(&self) -> &str {
        match self {
            Self::Webui => "webui",
            Self::Chat(c) => &c.sender_pid,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub plugin: String,
    pub operation: String,
    pub revision: String,
    #[serde(default)]
    pub patch: Map<String, Value>,
}
pub struct Management {
    db: PathBuf,
    plugins: Weak<PluginManager>,
    owner: Option<u64>,
    grants: RwLock<Vec<Grant>>,
    changes: tokio::sync::Mutex<()>,
    restart: tokio::sync::watch::Sender<bool>,
    restart_supported: bool,
    restarting: AtomicBool,
}
impl Management {
    pub async fn new(
        db: PathBuf,
        plugins: Arc<PluginManager>,
        owner: Option<u64>,
        restart_supported: bool,
    ) -> Result<(Arc<Self>, tokio::sync::watch::Receiver<bool>)> {
        let mut c = db::connect(&db).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM state_kv WHERE key=?")
            .bind(POLICY_KEY)
            .fetch_optional(&mut *c)
            .await?;
        let grants = raw
            .map(|s| serde_json::from_str::<Vec<Grant>>(&s))
            .transpose()?
            .unwrap_or_default();
        let (restart, receiver) = tokio::sync::watch::channel(false);
        Ok((
            Arc::new(Self {
                db,
                plugins: Arc::downgrade(&plugins),
                owner: owner.filter(|id| *id > 0),
                grants: RwLock::new(grants),
                changes: Default::default(),
                restart,
                restart_supported,
                restarting: AtomicBool::new(false),
            }),
            receiver,
        ))
    }
    fn qq(c: &ToolCtx) -> Option<u64> {
        // Privileged identity comes from adapter context, never memory links, nicknames or args.
        if c.chat_type != "private" || c.chat_id == "scheduled" {
            return None;
        }
        c.sender_pid
            .strip_prefix("p_")?
            .parse()
            .ok()
            .filter(|id| *id > 0)
    }
    pub fn visible(&self, c: &ToolCtx) -> bool {
        self.owner.is_some()
            && Self::qq(c).is_some_and(|id| {
                self.owner == Some(id)
                    || self
                        .grants
                        .read()
                        .unwrap()
                        .iter()
                        .any(|g| g.qq == id.to_string())
            })
    }
    fn authorize(&self, caller: &Caller, id: &str, action: &str) -> Result<()> {
        if matches!(caller, Caller::Webui) {
            return Ok(());
        }
        let Caller::Chat(ctx) = caller else {
            unreachable!()
        };
        ensure!(
            self.owner.is_some(),
            "尚未设置主人 ID，请先在机器人管理页配置"
        );
        let qq = Self::qq(ctx)
            .context("聊天管理仅限已授权 QQ 的私聊，不接受群身份或记忆库关联代替授权")?;
        if self.owner == Some(qq) {
            return Ok(());
        }
        let allowed = self.grants.read().unwrap().iter().any(|g| {
            g.qq == qq.to_string()
                && g.plugins.iter().any(|p| p == id)
                && match action {
                    "status" => true,
                    "configure" => g.configure,
                    "enable" | "disable" => g.toggle,
                    "reload" => g.reload,
                    _ => false,
                }
        });
        ensure!(allowed, "当前身份没有此插件操作权限");
        Ok(())
    }
    fn projection(plugin: Value) -> Value {
        // Chat never receives credentials, addresses, filesystem paths or raw provider errors.
        let mut config = Map::new();
        let mut fields = vec![];
        for f in plugin["config_fields"].as_array().into_iter().flatten() {
            let key = f["key"].as_str().unwrap_or("");
            if f["secret"] == true {
                continue;
            }
            if matches!(f["kind"].as_str(), Some("boolean" | "integer" | "choice")) {
                config.insert(key.into(), plugin["settings"]["config"][key].clone());
                fields.push(f.clone());
            }
        }
        json!({"name":plugin["name"],"display_name":plugin["display_name"],"version":plugin["version"],"revision":plugin["revision"],
            "enabled":plugin["enabled"],"loaded":plugin["loaded"],"updating":plugin["updating"],"pending_reload":plugin["pending_reload"],"pending_restart":plugin["pending_restart"],
            "has_error":plugin["error"].is_string(),"config":config,"config_fields":fields,"code_update_requires_restart":true})
    }
    pub fn status(&self, caller: &Caller) -> Result<Vec<Value>> {
        if let Caller::Chat(c) = caller {
            ensure!(
                self.visible(c),
                "没有机器人管理权限，请由主人授权后私聊使用"
            )
        }
        Ok(self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .list()
            .into_iter()
            .filter(|p| {
                self.authorize(caller, p["name"].as_str().unwrap_or(""), "status")
                    .is_ok()
            })
            .map(Self::projection)
            .collect())
    }
    pub fn policy(&self) -> Value {
        json!({"owner_qq":self.owner.map(|v|v.to_string()),"grants":self.grants.read().unwrap().clone(),"restart_supported":self.restart_supported,"restarting":self.restarting.load(Ordering::Acquire),"chat_private_only":true,"webui_role":"owner","plugin_install_supported":false,"plugin_code_update_supported":false})
    }
    pub async fn set_grants(&self, grants: Vec<Grant>) -> Result<()> {
        ensure!(grants.len() <= 64, "最多配置64名运维管理员");
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<_> = self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .list()
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_owned))
            .collect();
        for g in &grants {
            let qq =
                g.qq.parse::<u64>()
                    .ok()
                    .filter(|v| *v > 0 && v.to_string() == g.qq)
                    .context("管理员 QQ 必须是完整数字 ID")?;
            ensure!(
                Some(qq) != self.owner && seen.insert(qq),
                "不能重复授权或将主人列入运维管理员"
            );
            ensure!(
                !g.plugins.is_empty()
                    && g.plugins.len() <= ids.len()
                    && g.plugins.iter().all(|p| ids.contains(p)),
                "请选择已注册插件"
            );
        }
        let _lock = self.changes.lock().await;
        let mut c = db::connect(&self.db).await?;
        let mut tx = c.begin().await?;
        sqlx::query("INSERT INTO state_kv(key,value,updated_at) VALUES(?,?,unixepoch()) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at")
            .bind(POLICY_KEY).bind(serde_json::to_string(&grants)?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO events(kind,payload,ts) VALUES('ManagementAction',?,unixepoch())").bind(json!({"actor":"webui","action":"permissions","status":"applied","administrator_count":grants.len()}).to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        *self.grants.write().unwrap() = grants;
        Ok(())
    }
    async fn audit(
        &self,
        caller: &Caller,
        id: &str,
        op: &str,
        fields: Vec<String>,
        status: &str,
    ) -> Result<i64> {
        let mut c = db::connect(&self.db).await?;
        Ok(sqlx::query_scalar("INSERT INTO events(kind,payload,ts) VALUES('ManagementAction',?,unixepoch()) RETURNING id")
            .bind(json!({"actor":caller.actor(),"action":op,"plugin":id,"fields":fields,"status":status}).to_string()).fetch_one(&mut *c).await?)
    }
    pub async fn apply(&self, caller: &Caller, b: Action) -> Result<Value> {
        if let Err(e) = self.authorize(caller, &b.plugin, &b.operation) {
            self.audit(caller, &b.plugin, &b.operation, vec![], "denied")
                .await?;
            return Err(e);
        }
        ensure!(
            ["configure", "enable", "disable", "reload"].contains(&b.operation.as_str()),
            "未知管理操作"
        );
        ensure!(
            b.operation == "configure" || b.patch.is_empty(),
            "启停和重载操作不能附带配置修改"
        );
        let _lock = self.changes.lock().await;
        self.authorize(caller, &b.plugin, &b.operation)?;
        let p = self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .list()
            .into_iter()
            .find(|p| p["name"] == b.plugin)
            .context("插件未注册在当前版本")?;
        let safe = Self::projection(p);
        for (key, value) in &b.patch {
            ensure!(
                safe["config"].get(key).is_some(),
                "此字段不可通过聊天管理；凭据、地址和路径请使用插件配置页"
            );
            ensure!(
                value.is_boolean() || value.is_i64() || value.is_u64() || value.is_string(),
                "配置值类型无效"
            );
        }
        if b.operation == "configure" {
            ensure!(!b.patch.is_empty(), "请提供要修改的配置字段")
        }
        let revision = b
            .revision
            .parse::<u64>()
            .context("请先查询插件状态并提供其 revision")?;
        let fields = b.patch.keys().cloned().collect();
        let record = self
            .audit(caller, &b.plugin, &b.operation, fields, "accepted")
            .await?;
        let result = self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .managed_change(
                &b.plugin,
                revision,
                b.patch,
                match b.operation.as_str() {
                    "enable" => Some(true),
                    "disable" => Some(false),
                    _ => None,
                },
                b.operation == "reload",
            )
            .await;
        let state = match &result {
            Ok(true) => "applying",
            Ok(false) => "applied",
            Err(_) => "failed",
        };
        let mut c = db::connect(&self.db).await?;
        sqlx::query("UPDATE events SET payload=json_set(payload,'$.status',?) WHERE id=? AND kind='ManagementAction'").bind(state).bind(record).execute(&mut *c).await?;
        result.map_err(|_| {
            anyhow::anyhow!("插件操作未能生效，请重新查询状态；详细错误在插件管理页查看")
        })?;
        let current = self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .list()
            .into_iter()
            .find(|p| p["name"] == b.plugin)
            .map(Self::projection);
        if let Some(plugin) = &current {
            sqlx::query("UPDATE events SET payload=json_set(payload,'$.revision',?) WHERE id=? AND kind='ManagementAction'")
                .bind(plugin["revision"].as_str()).bind(record).execute(&mut *c).await?;
        }
        Ok(
            json!({"status":state,"plugin":current,"audit_id":record,"message":if state=="applying"{"已提交，等待当前调用完成，请再次查询确认是否生效"}else{"操作已生效"}}),
        )
    }
    pub async fn history(&self) -> Result<Vec<Value>> {
        let mut c = db::connect(&self.db).await?;
        let rows=sqlx::query("SELECT id,ts,payload FROM events WHERE kind='ManagementAction' ORDER BY id DESC LIMIT 100").fetch_all(&mut *c).await?;
        let plugins = self
            .plugins
            .upgrade()
            .context("plugin host unavailable")?
            .list();
        rows.iter().map(|r|{
            let mut record=serde_json::from_str::<Value>(&r.try_get::<String,_>("payload")?)?;
            if record["status"]=="applying" {
                if let Some(p)=plugins.iter().find(|p|p["name"]==record["plugin"]) {
                    if p["revision"]!=record["revision"] {record["status"]=json!("superseded")}
                    else if p["updating"]!=true {record["status"]=json!(if p["pending_reload"]==true||p["pending_restart"]==true||p["error"].is_string(){"attention"}else{"applied"})}
                }
            }
            Ok(json!({"id":r.try_get::<i64,_>("id")?,"ts":r.try_get::<i64,_>("ts")?,"record":record}))
        }).collect()
    }
    pub async fn request_restart(&self, caller: &Caller) -> Result<()> {
        self.authorize(caller, "application", "restart")?;
        let _lock = self.changes.lock().await;
        ensure!(
            self.restart_supported,
            "未配置进程守护重启，请由部署管理员配置后使用"
        );
        ensure!(self.restart.receiver_count() > 0, "运行时重启监听器未就绪");
        ensure!(!self.restarting.load(Ordering::Acquire), "重启已在进行中");
        self.audit(caller, "application", "restart", vec![], "accepted")
            .await?;
        ensure!(
            self.restarting
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "重启已在进行中"
        );
        self.restart.send(true).context("运行时重启监听器未就绪")?;
        Ok(())
    }
}

pub struct RestartSkill(pub Arc<Management>);
#[async_trait]
impl crate::skills::Skill for RestartSkill {
    fn def(&self) -> crate::skills::SkillDef {
        crate::skills::SkillDef {
            name: "management.restart",
            description: "仅主人私聊明确发送 /重启云团 时重启云团应用，不重启 QQ 登录容器。",
            prompt_template: String::new(),
            tools: vec![],
            role_hint: None,
        }
    }
    fn commands(&self) -> &'static [&'static str] {
        &["/重启云团"]
    }
    fn accepts_empty_command(&self) -> bool {
        true
    }
    async fn invoke(&self, c: &ToolCtx, v: Value) -> Result<ToolOutput> {
        ensure!(
            v["explicit_command"] == true
                && v["original_user_text"]
                    .as_str()
                    .is_some_and(|s| s.trim() == "/重启云团"),
            "重启必须由主人私聊明确发送 /重启云团"
        );
        self.0.request_restart(&Caller::Chat(c.clone())).await?;
        Ok(ToolOutput {
            summary: "云团正在收尾并重启，QQ 登录容器保持运行，请稍后再试".into(),
            data: json!({"status":"restarting"}),
            artifacts: vec![],
        })
    }
}

pub struct ManagementTool {
    pub service: Arc<Management>,
    pub write: bool,
}
#[async_trait]
impl Tool for ManagementTool {
    fn name(&self) -> &'static str {
        if self.write {
            "management.plugins_apply"
        } else {
            "management.plugins_status"
        }
    }
    fn description(&self) -> &'static str {
        if self.write {
            "管理员管理插件：先查询状态，再用 revision 修改可用配置、启停或重载；仅私聊，不能管理凭据或安装代码。核对返回状态，applying需要再查询。"
        } else {
            "主人或授权管理员查看插件状态和可修改参数，返回 revision。查询实际加载、待重载及需重启状态，不返回密钥。"
        }
    }
    fn policy(&self) -> ToolPolicy {
        ToolPolicy {
            effect: if self.write {
                ToolEffect::ExternalWrite
            } else {
                ToolEffect::ReadOnly
            },
            ..Default::default()
        }
    }
    fn available_in_chat(&self, c: &ToolCtx) -> bool {
        self.service.visible(c)
    }
    fn parameters_schema(&self) -> Value {
        if !self.write {
            return json!({"type":"object","additionalProperties":false,"properties":{}});
        }
        json!({"type":"object","additionalProperties":false,"required":["plugin","operation","revision"],"properties":{"plugin":{"type":"string","maxLength":64},"operation":{"type":"string","enum":["configure","enable","disable","reload"]},"revision":{"type":"string"},"patch":{"type":"object"}}})
    }
    async fn call(&self, c: &ToolCtx, v: Value) -> Result<ToolOutput> {
        ensure!(self.service.visible(c), "没有机器人管理权限");
        let caller = Caller::Chat(c.clone());
        if let Some(id) = &c.task_id {
            let mut conn = db::connect(&self.service.db).await?;
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks t JOIN task_runtime r USING(task_id) WHERE t.task_id=? AND t.created_by_pid=? AND t.chat_id=? AND r.chat_type=? AND t.state='running')").bind(id).bind(&c.sender_pid).bind(&c.chat_id).bind(&c.chat_type).fetch_one(&mut *conn).await?;
            ensure!(owned, "管理工具调用身份与任务来源不匹配");
        }
        let data = if self.write {
            self.service
                .apply(&caller, serde_json::from_value(v)?)
                .await?
        } else {
            json!({"plugins":self.service.status(&caller)?})
        };
        Ok(ToolOutput {
            summary: if self.write {
                data["message"].as_str().unwrap_or("管理操作已返回").into()
            } else {
                format!(
                    "已查询{}个授权插件，状态和可用参数见结果",
                    data["plugins"].as_array().map_or(0, Vec::len)
                )
            },
            data,
            artifacts: vec![],
        })
    }
}
