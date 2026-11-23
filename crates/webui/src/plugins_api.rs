//! Runtime plugin catalog and configuration reload.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
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

#[derive(Deserialize)]
pub struct HistoryQuery {
    from_ts: Option<i64>,
    until_ts: Option<i64>,
}

type HistoryRow = (
    String, String, String, String, Option<String>, Option<i64>, Option<String>, String, i64,
    Option<i64>,
);

fn history_error(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(%error, "读取插件调用记录失败");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error":"读取插件调用记录失败"})))
}

/// Read persisted Skill and Agent Tool receipts, excluding arguments and output content.
pub async fn history(
    State(state): State<AppState>,
    Path(plugin): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    if plugin.is_empty() || plugin.len() > 64 || !plugin.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
        return Err(err("插件名称无效"));
    }
    let from = query.from_ts.unwrap_or(i64::MIN);
    let until = query.until_ts.unwrap_or(i64::MAX);
    if from >= until {
        return Err(err("日期范围无效"));
    }
    let mut conn = state.open_db().await.map_err(history_error)?;
    let rows = sqlx::query_as::<_, HistoryRow>(
        "WITH recent AS (
            SELECT CAST(id AS TEXT) AS id,kind,name,chat_id,chat_type,anchor_id,NULL AS task_id,state AS status,started_at,
                   MAX(0,(finished_at-started_at)*1000) AS elapsed_ms,id AS sequence
            FROM capability_invocations WHERE kind='skill' AND name>=?1 AND name<?2 AND started_at>=?3 AND started_at<?4
            UNION ALL
            SELECT i.id,'tool',i.tool_name,t.chat_id,rt.chat_type,rt.anchor_id,i.task_id,COALESCE(r.status,i.state),i.started_at,
                   COALESCE(r.elapsed_ms,MAX(0,(i.finished_at-i.started_at)*1000)),i.rowid
            FROM task_invocations i JOIN tasks t ON t.task_id=i.task_id
            LEFT JOIN task_runtime rt ON rt.task_id=i.task_id
            LEFT JOIN tool_invocation_records r ON r.invocation_id=i.id
            WHERE i.tool_name>=?1 AND i.tool_name<?2 AND i.started_at>=?3 AND i.started_at<?4
        ) SELECT id,kind,name,chat_id,chat_type,anchor_id,task_id,status,started_at,elapsed_ms
          FROM recent ORDER BY started_at DESC,kind,sequence DESC LIMIT 50",
    )
    .bind(format!("{plugin}."))
    .bind(format!("{plugin}/"))
    .bind(from)
    .bind(until)
    .fetch_all(&mut *conn)
    .await
    .map_err(history_error)?;
    let items: Vec<_> = rows.into_iter().map(|(id,kind,name,chat_id,chat_type,anchor_id,task_id,status,started_at,elapsed_ms)| {
        json!({"id":id,"kind":kind,"name":name,"chat_id":chat_id,"chat_type":chat_type,
            "anchor_id":anchor_id,"task_id":task_id,"status":status,"started_at":started_at,"elapsed_ms":elapsed_ms})
    }).collect();
    Ok(Json(json!({"items":items})))
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
