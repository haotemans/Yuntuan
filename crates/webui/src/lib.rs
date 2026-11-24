//! yuantuan-webui：axum API + WS 推送 + 内嵌占位前端。
//! 定稿依据：docs/design/runtime-design.md 第四章、docs/design/architecture-v0.1.md 第十五章安全小节。

mod auth;
mod backup_api;
mod config_api;
mod connector_auth;
mod dashboard;
mod events_api;
mod logs_api;
mod llm_test;
mod mcp_api;
mod memes;
mod memories;
mod overview;
mod personality;
mod plugins_api;
mod relations;
mod runtime_api;
mod agent_api;
mod management_api;
mod schedule_api;
mod state;
mod static_files;
mod tasks_api;
mod ws;

use anyhow::{Context, Result};
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::trace::TraceLayer;

pub use state::{AppState, Extras};

/// 组装路由并启动 HTTP 服务（阻塞至进程结束）
pub async fn serve(db_path: PathBuf, host: &str, port: u16, extras: Extras) -> Result<()> {
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("非法监听地址: {host}:{port}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("WebUI 监听失败: {addr}"))?;
    serve_on(listener, db_path, extras).await
}

/// 使用已有的 TCP 监听器启动同一套 WebUI 路由。
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    db_path: PathBuf,
    extras: Extras,
) -> Result<()> {
    let state = AppState::new(db_path, extras);

    // /api/* 中除 login 外一律过 token 校验
    let protected = Router::new()
        .route("/api/schedules", get(schedule_api::tasks).post(schedule_api::add_task))
        .route("/api/schedules/{id}", post(schedule_api::toggle_task).delete(schedule_api::remove_task))
        .route("/api/schedules/{id}/time", post(schedule_api::set_time))
        .route("/api/schedules/{id}/run", post(schedule_api::run_now))
        .route("/api/overview", get(overview::overview))
        .route("/api/platforms", get(overview::platforms))
        .route("/api/dashboard", get(dashboard::dashboard))
        .route("/api/events", get(events_api::list))
        .route("/api/logs", get(logs_api::logs))
        .route("/api/runtime/status", get(logs_api::status))
        .route("/api/runtime/agent", get(agent_api::status))
        .route("/api/management", get(management_api::status))
        .route("/api/management/permissions", post(management_api::permissions))
        .route("/api/management/owner", post(management_api::owner))
        .route("/api/management/plugins", post(management_api::apply))
        .route("/api/runtime/bash/check", post(agent_api::check_bash))
        .route("/api/agent-schedules", get(agent_api::list).post(agent_api::create))
        .route("/api/agent-schedules/{id}", post(agent_api::toggle).delete(agent_api::remove))
        .route("/api/agent-schedules/{id}/time", post(agent_api::time))
        .route("/api/agent-schedules/{id}/run", post(agent_api::run))
        .route("/api/agent-schedules/{id}/history", get(agent_api::history))
        .route("/api/decisions/status", get(events_api::decision_status))
        .route("/api/memes", get(memes::list))
        .route("/api/memes/batch", post(memes::batch))
        .route("/api/memes/{id}/approve", post(memes::approve))
        .route("/api/memes/{id}/category", post(memes::category))
        .route("/api/memes/{id}/reject", post(memes::reject))
        .route("/api/meme-file/{id}", get(memes::file))
        .route("/api/memories", get(memories::memories))
        .route("/api/memories/people", get(memories::people))
        .route(
            "/api/memories/people/{id}/refresh",
            post(memories::refresh_profile),
        )
        .route(
            "/api/memories/people/{id}/activity",
            get(memories::activity::status),
        )
        .route(
            "/api/memories/people/{id}/profile-history",
            get(memories::activity::profile_history),
        )
        .route(
            "/api/memories/people/{id}/messages",
            get(memories::activity::messages),
        )
        .route("/api/memories/conversations", get(memories::conversations))
        .route("/api/memories/{id}/source", get(memories::source))
        .route("/api/summaries", get(memories::summaries))
        .route("/api/relations", get(relations::relations))
        .route("/api/relations/person/{id}", get(relations::person_detail))
        .route(
            "/api/config",
            get(config_api::get_config).post(config_api::post_config),
        )
        .route("/api/config/restart", post(config_api::restart))
        .route("/api/llm/test", post(llm_test::test))
        .route("/api/llm/models", get(llm_test::list_models))
        .route("/api/llm/models/probe", post(llm_test::probe_models))
        .route("/api/personality/defaults", get(personality::defaults))
        .route(
            "/api/personality/versions",
            get(personality::list).post(personality::create),
        )
        .route("/api/personality/versions/{no}", get(personality::get_one))
        .route(
            "/api/personality/rollback/{no}",
            post(personality::rollback),
        )
        .route(
            "/api/capabilities",
            get(runtime_api::capabilities)
                .post(runtime_api::toggle)
                .patch(runtime_api::patch_capability),
        )
        .route("/api/tasks/{id}/runtime", get(runtime_api::task))
        .route("/api/tasks/{id}/control", post(runtime_api::control))
        .route("/api/tasks", get(tasks_api::list))
        .route("/api/tasks/{id}/events", get(tasks_api::events))
        .route("/api/kb/status", get(kb_status))
        .route("/api/backup/run", post(backup_api::run))
        .route("/api/backup/list", get(backup_api::list))
        .route("/api/backup/restore", post(backup_api::restore))
        .route("/api/backup/file/{name}", get(backup_api::file))
        .route("/api/plugins/list", get(plugins_api::list))
        .route("/api/plugins/{name}/history", get(plugins_api::history))
        .route("/api/plugins/toggle", post(plugins_api::toggle))
        .route("/api/plugins/config", post(plugins_api::save_config))
        .route(
            "/api/plugins/connection/check",
            post(plugins_api::check_connection),
        )
        .route(
            "/api/plugins/auth/{name}",
            get(connector_auth::status).post(connector_auth::start),
        )
        .route(
            "/api/plugins/auth/{name}/complete",
            post(connector_auth::complete),
        )
        .route(
            "/api/plugins/quark/watches",
            get(schedule_api::list).post(schedule_api::add),
        )
        .route(
            "/api/plugins/quark/watches/{id}",
            post(schedule_api::toggle).delete(schedule_api::remove),
        )
        .route(
            "/api/plugins/quark/watches/{id}/check",
            post(schedule_api::check_now),
        )
        .route("/api/mcp/list", get(mcp_api::list))
        .route("/api/mcp/save", post(mcp_api::save))
        .route("/api/mcp/call", post(mcp_api::call))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_session,
        ));
    let api = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/status", get(auth::status))
        .merge(protected);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws::handler)) // token 走 query param（浏览器 WS 无自定义头），handler 内校验
        .merge(api)
        .fallback(static_files::serve)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = listener.local_addr().context("读取 WebUI 监听地址失败")?;
    tracing::info!(%addr, "WebUI 开始监听");
    axum::serve(listener, app)
        .await
        .context("WebUI 服务异常退出")
}

async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn kb_status() -> axum::Json<serde_json::Value> {
    // 知识库为二期可插拔功能（架构十五章扩展模型），默认关闭
    axum::Json(serde_json::json!({ "enabled": false, "docs": 0 }))
}
