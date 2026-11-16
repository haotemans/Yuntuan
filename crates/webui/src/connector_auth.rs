//! Authenticated administrator onboarding. Credentials never enter chat, Agent
//! tools, query strings, logs or API responses. Jobs are bounded and session-bound.
use crate::AppState;
use anyhow::{ensure, Context};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use rand::{distr::Alphanumeric, Rng};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use yuantuan_plugin_quark_drive::auth as quark;

type Error = (StatusCode, Json<Value>);
fn bad(message: impl ToString) -> Error {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok":false,"error":message.to_string()})),
    )
}
const TTL: Duration = Duration::from_secs(600);
#[derive(Clone, Default)]
pub(crate) struct AuthState(Arc<Mutex<HashMap<String, Job>>>);
struct Job {
    id: String,
    owner: String,
    expires: Instant,
    phase: &'static str,
    message: String,
    url: Option<String>,
    script: Option<PathBuf>,
}
impl Job {
    fn busy(&self) -> bool {
        matches!(self.phase, "preparing" | "connecting")
    }
    fn view(&self) -> Value {
        json!({"phase":self.phase,"message":self.message,"authorization_url":self.url,
            "expires_in":self.expires.saturating_duration_since(Instant::now()).as_secs()})
    }
}
impl AuthState {
    fn reserve(&self, name: &str, owner: String) -> anyhow::Result<String> {
        let mut jobs = self.0.lock().unwrap();
        jobs.retain(|_, j| j.expires > Instant::now());
        if let Some(old) = jobs.get(name) {
            ensure!(
                !old.busy() && (old.owner == owner || old.phase != "waiting"),
                "已有授权操作正在进行，请等待完成或过期"
            );
        }
        let id: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        jobs.insert(
            name.into(),
            Job {
                id: id.clone(),
                owner,
                expires: Instant::now() + TTL,
                phase: "preparing",
                message: "正在准备连接，请稍候".into(),
                url: None,
                script: None,
            },
        );
        Ok(id)
    }
    fn finish(&self, name: &str, id: &str, result: anyhow::Result<Option<(String, PathBuf)>>) {
        let mut jobs = self.0.lock().unwrap();
        let Some(job) = jobs.get_mut(name).filter(|j| j.id == id) else {
            return;
        };
        job.expires = Instant::now() + TTL;
        match result {
            Ok(Some((url, script))) => {
                job.phase = "waiting";
                job.url = Some(url);
                job.script = Some(script);
                job.message = "打开夸克授权页，完成登录后回填跳转链接或授权码".into();
            }
            Ok(None) => {
                job.phase = "connected";
                job.message = "账号连接检查已通过，配置已保存并启用".into();
                job.url = None;
                job.script = None;
            }
            Err(error) => {
                job.phase = "error";
                job.message = error.to_string();
                job.url = None;
                job.script = None;
            }
        }
    }
}
fn owner(headers: &HeaderMap) -> Result<String, Error> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_owned)
        .ok_or_else(|| bad("请先登录管理页面"))
}
fn known(state: &AppState, name: &str) -> Result<Value, Error> {
    if !matches!(name, "quark-drive" | "tencent-docs") {
        return Err(bad("此插件没有账号授权入口"));
    }
    state
        .extras
        .plugin_manager
        .as_ref()
        .ok_or_else(|| bad("插件管理器未接入"))?
        .list()
        .into_iter()
        .find(|p| p["name"] == name)
        .ok_or_else(|| bad("该插件未编译安装"))
}
pub async fn status(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    known(&state, &name)?;
    let owner = owner(&headers)?;
    let mut jobs = state.connector_auth.0.lock().unwrap();
    jobs.retain(|_, j| j.expires > Instant::now());
    let view = match jobs.get(&name) {
        Some(j) if j.owner == owner => j.view(),
        Some(j) if j.busy() || j.phase == "waiting" => {
            json!({"phase":"occupied","message":"另一个管理会话正在授权，请稍后再试"})
        }
        _ => json!({"phase":"idle","message":""}),
    };
    Ok(Json(view))
}
pub async fn start(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    let plugin = known(&state, &name)?;
    if name != "quark-drive" {
        return Err(bad("请打开腾讯文档官方授权页并回填个人 Token"));
    }
    let id = state
        .connector_auth
        .reserve(&name, owner(&headers)?)
        .map_err(bad)?;
    let configured = plugin["settings"]["config"]["cli_script"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    tokio::spawn(async move {
        let work = async {
            let root = state.db_path.parent().context("数据目录不可用")?;
            let script = quark::prepare(root, &configured).await?;
            match quark::begin(&script).await? {
                quark::Login::Link(url) => Ok(Some((url, script))),
                quark::Login::Authorized => {
                    save_quark(&state, &script).await?;
                    Ok(None)
                }
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(240), work)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("授权准备超时，请检查网络后重新发起")));
        state.connector_auth.finish(&name, &id, result);
    });
    Ok(Json(json!({"ok":true,"phase":"preparing"})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteBody {
    value: String,
}
pub async fn complete(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CompleteBody>,
) -> Result<Json<Value>, Error> {
    known(&state, &name)?;
    let owner = owner(&headers)?;
    if name == "tencent-docs" {
        let token = body.value.trim().to_owned();
        ensure_token(&token).map_err(bad)?;
        let id = state.connector_auth.reserve(&name, owner).map_err(bad)?;
        tokio::spawn(async move {
            let work = async {
                // Fixed official destination, only tools/list; no document writes.
                let client = yuantuan_mcp_http::HttpMcpClient::new(
                    "https://docs.qq.com/openapi/mcp",
                    &token,
                )?;
                let catalog = client.list_tools().await?;
                ensure!(
                    !catalog.is_empty(),
                    "腾讯文档未返回可用工具，请检查个人 Token"
                );
                let patch = json!({"token":token,"token_env":"","endpoint":"https://docs.qq.com/openapi/mcp"});
                state
                    .extras
                    .plugin_manager
                    .as_ref()
                    .context("插件管理器不可用")?
                    .patch_config(&name, patch.as_object().unwrap().clone(), Some(true))
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("连接检查通过，但配置未能应用；请检查插件状态后重新保存")
                    })?;
                Ok(None)
            };
            let result = tokio::time::timeout(Duration::from_secs(180), work)
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("腾讯文档连接检查超时，请检查网络后重试")));
            state.connector_auth.finish(&name, &id, result);
        });
    } else {
        let code = quark::authorization_code(&body.value).map_err(bad)?;
        let (id, script) = {
            let mut jobs = state.connector_auth.0.lock().unwrap();
            let job = jobs
                .get_mut(&name)
                .filter(|j| j.owner == owner && j.phase == "waiting" && j.expires > Instant::now())
                .ok_or_else(|| bad("授权请求已过期或已使用，请重新打开授权入口"))?;
            let script = job.script.take().ok_or_else(|| bad("授权组件未就绪"))?;
            job.phase = "connecting";
            job.message = "正在保存授权并检查账号".into();
            job.url = None;
            job.expires = Instant::now() + TTL;
            (job.id.clone(), script)
        };
        tokio::spawn(async move {
            let work = async {
                quark::complete(&script, &code).await?;
                save_quark(&state, &script).await?;
                Ok(None)
            };
            let result = tokio::time::timeout(Duration::from_secs(240), work)
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "授权检查超时，结果待核实；请重新打开授权入口检查现有登录"
                    ))
                });
            state.connector_auth.finish(&name, &id, result);
        });
    }
    Ok(Json(json!({"ok":true,"phase":"connecting"})))
}
fn ensure_token(token: &str) -> anyhow::Result<()> {
    ensure!(
        !token.is_empty() && token.len() <= 8192 && !token.chars().any(char::is_control),
        "请回填有效的腾讯文档个人 Token"
    );
    Ok(())
}
async fn save_quark(state: &AppState, script: &std::path::Path) -> anyhow::Result<()> {
    let patch = json!({"cli_script":script.to_string_lossy()});
    state
        .extras
        .plugin_manager
        .as_ref()
        .context("插件管理器不可用")?
        .patch_config(
            "quark-drive",
            patch.as_object().unwrap().clone(),
            Some(true),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!("账号授权已保存，但插件配置未能应用；请检查插件状态后重新连接")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_jobs_are_single_flight_session_bound_expire_and_do_not_expose_codes() {
        let state = AuthState::default();
        let id = state.reserve("quark-drive", "session-a".into()).unwrap();
        assert!(state.reserve("quark-drive", "session-a".into()).is_err());
        state.finish(
            "quark-drive",
            &id,
            Ok(Some((
                "https://pan.quark.cn/?client_id=x".into(),
                PathBuf::from("fixture"),
            ))),
        );
        assert!(state.reserve("quark-drive", "session-b".into()).is_err());
        let view = state.0.lock().unwrap()["quark-drive"].view();
        assert!(!view.to_string().contains("session-a"));
        assert!(!view.to_string().contains("fixture"));
        state
            .0
            .lock()
            .unwrap()
            .get_mut("quark-drive")
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        let next = state.reserve("quark-drive", "session-b".into()).unwrap();
        state.finish("quark-drive", &id, Err(anyhow::anyhow!("stale")));
        assert_eq!(state.0.lock().unwrap()["quark-drive"].id, next);
        assert_eq!(state.0.lock().unwrap()["quark-drive"].phase, "preparing");
    }
}
