//! Runtime plugin catalog and configuration reload.
use crate::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use yuantuan_core::plugins::PluginSettings;

type ApiError = (StatusCode, Json<Value>);
fn err(message: impl ToString) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok":false,"error":message.to_string()})),
    )
}
pub async fn list(State(state): State<AppState>) -> Json<Value> {
    Json(
        json!({"ok":true,"plugins":state.extras.plugin_manager.as_ref().map(|p| p.list()).unwrap_or_default(),
        "skill_catalog": state.extras.skill_registry.describe_for_decision().iter()
            .map(|(name, description)| json!({"name":name,"description":description})).collect::<Vec<_>>() }),
    )
}

/// Administrator-only, read-only connector probes through the normal registered Tool.
pub async fn check_connection(
    State(state): State<AppState>,
    Json(body): Json<ConnectionBody>,
) -> Result<Json<Value>, ApiError> {
    use yuantuan_core::{
        capabilities,
        tools::{ToolCtx, ToolEffect},
    };
    let name = match body.name.as_str() {
        "tencent-docs" => "tencent-docs.catalog",
        "quark-drive" => "quark-drive.account",
        _ => return Err(err("此插件没有注册连接检查")),
    };
    let tool = state
        .extras
        .tools_registry
        .get(name)
        .ok_or_else(|| err("请先保存配置并启用插件"))?;
    let ctx = ToolCtx {
        task_id: None,
        chat_id: "plugin-check".into(),
        chat_type: "private".into(),
        sender_pid: "webui".into(),
        locale: Some("zh-CN".into()),
    };
    if tool.policy().effect != ToolEffect::ReadOnly || !tool.available_in_chat(&ctx) {
        return Err(err("连接检查不在插件允许的只读范围内"));
    }
    capabilities::check_tool(
        &state.db_path,
        name,
        &ctx,
        &tool.parameters_schema(),
        &json!({}),
    )
    .await
    .map_err(err)?;
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tool.call(&ctx, json!({})),
    )
    .await
    .map_err(|_| err("连接检查超时，请检查授权与网络"))?
    .map_err(err)?;
    Ok(Json(json!({"ok":true,"summary":out.summary})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionBody {
    name: String,
}
#[derive(Deserialize)]
pub struct ToggleBody {
    name: String,
    enabled: bool,
}
pub async fn toggle(
    State(state): State<AppState>,
    Json(body): Json<ToggleBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let manager = state
        .extras
        .plugin_manager
        .as_ref()
        .ok_or_else(|| err("插件宿主未启动"))?;
    let applying = manager
        .toggle_deferred(&body.name, body.enabled)
        .await
        .map_err(err)?;
    Ok((
        if applying {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(
            json!({"ok":true,"applying":applying,"message":if applying { "正在等待当前调用完成后应用" } else { "插件状态已生效" }}),
        ),
    ))
}
#[derive(Deserialize)]
pub struct ConfigBody {
    name: String,
    settings: PluginSettings,
}
pub async fn save_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if let Ok(settings) = serde_json::to_value(&body.settings) {
        state.extras.runtime_logs.register_secrets(&settings);
    }
    let manager = state
        .extras
        .plugin_manager
        .as_ref()
        .ok_or_else(|| err("插件宿主未启动"))?;
    let applying = manager
        .save_deferred(&body.name, body.settings)
        .await
        .map_err(err)?;
    Ok((
        if applying {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(
            json!({"ok":true,"applying":applying,"message":if applying { "配置已提交，正在等待当前调用完成" } else { "配置已保存并生效" }}),
        ),
    ))
}
