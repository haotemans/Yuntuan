//! Admin-only Agent readiness, sandbox preflight and persistent schedules.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use yuantuan_core::{llm::Role, scheduler::agents};

type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn actor(s: &AppState) -> String {
    s.extras
        .account_owner
        .map(|id| format!("p_{id}"))
        .unwrap_or_else(|| "scheduler".into())
}
fn ready(s: &AppState) -> bool {
    s.extras
        .llm_slot
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|g| g.role(Role::AgentExec).is_some())
}

pub async fn status(State(s): State<AppState>) -> Response {
    let registered = s.extras.tools_registry.get("sandbox_exec").is_some();
    let ctx = agents::context(&actor(&s));
    let allowed = registered
        && yuantuan_core::capabilities::allowed(&s.db_path, "sandbox_exec", "tool", &ctx)
            .await
            .map_err(err)?;
    Ok(Json(
        json!({"model_configured":ready(&s),"bash":{"enabled":s.extras.sandbox.enabled,"registered":registered,
        "state":if !s.extras.sandbox.enabled { "disabled" } else if !registered { "unavailable" } else if !allowed { "restricted" } else { "ready" },
        "message":if !s.extras.sandbox.enabled { "Bash 未启用；部署沙箱后在运行参数中配置 sandbox，重启生效" } else if !registered { "Bash 未注册，请检查启动日志中的沙箱错误" } else if !allowed { "Bash 已注册，但未允许定时任务会话；请在插件工具权限中设置 private:scheduled" } else { "Bash 已注册且允许定时任务；可检查当前 Docker 环境" },
        "network":match s.extras.sandbox.network { yuantuan_core::tools::sandbox::SandboxNetwork::None=>"none",_=>"bridge" },
        "timeout_secs":s.extras.sandbox.timeout_secs},"tools":agents::options(&s.db_path,&s.extras.tools_registry,&actor(&s)).await.map_err(err)?}),
    ))
}

pub async fn check_bash(State(s): State<AppState>) -> Response {
    let _guard = s.extras.bash_probe_lock.try_lock().map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(json!({"error":"环境检查正在进行"})),
        )
    })?;
    let sandbox = yuantuan_core::tools::sandbox::SandboxExecTool::new(s.extras.sandbox.clone())
        .map_err(err)?;
    // No command is executed, no image is pulled, and no server configuration is changed.
    let result = sandbox.check_available().await;
    if let Err(error) = &result {
        tracing::warn!(%error,"Bash 环境预检未通过");
    }
    Ok(Json(
        json!({"ok":result.is_ok(),"state":if result.is_ok(){"ready"}else{"environment_missing"},
        "message":if result.is_ok(){"Docker 与 Linux 沙箱镜像检查通过；这不代表已执行命令"}else{"Docker 或沙箱镜像不可用，请检查 Docker CLI、专用 daemon 连接、socket 权限与镜像；详情见运行日志"}}),
    ))
}

pub async fn list(State(s): State<AppState>) -> Response {
    Ok(Json(
        json!({"tasks":agents::list(&s.db_path).await.map_err(err)?}),
    ))
}
pub async fn create(State(s): State<AppState>, Json(draft): Json<agents::Draft>) -> Response {
    if !ready(&s) {
        return Err(err("请先在模型服务中配置 Agent 执行模型（agent_exec）"));
    }
    let id = agents::register(&s.db_path, &s.extras.tools_registry, &actor(&s), draft)
        .await
        .map_err(err)?;
    Ok(Json(json!({"id":id})))
}
#[derive(Deserialize)]
pub struct Toggle {
    enabled: bool,
}
#[derive(Deserialize)]
pub struct Time {
    daily_time: String,
}
pub async fn toggle(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<Toggle>,
) -> Response {
    agents::update(&s.db_path, id, Some(b.enabled), None, false)
        .await
        .map_err(err)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn time(State(s): State<AppState>, Path(id): Path<i64>, Json(b): Json<Time>) -> Response {
    agents::update(&s.db_path, id, None, Some(&b.daily_time), false)
        .await
        .map_err(err)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> Response {
    agents::update(&s.db_path, id, None, None, true)
        .await
        .map_err(err)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn run(State(s): State<AppState>, Path(id): Path<i64>) -> Response {
    let id = agents::trigger(
        &s.db_path,
        &s.extras.tools_registry,
        &s.extras.bus,
        id,
        true,
        ready(&s),
    )
    .await
    .map_err(err)?;
    Ok(Json(
        json!({"task_id":id,"message":"已进入任务队列，尚未执行完成"}),
    ))
}
pub async fn history(State(s): State<AppState>, Path(id): Path<i64>) -> Response {
    Ok(Json(
        json!({"runs":agents::history(&s.db_path,id).await.map_err(err)?}),
    ))
}
