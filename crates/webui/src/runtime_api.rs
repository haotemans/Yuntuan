//! Authenticated controls and persistent capability/task diagnostics.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use yuantuan_core::db::SqliteExt;
type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}

pub async fn capabilities(State(s): State<AppState>) -> Response {
    for name in s.extras.tools_registry.names() {
        if let Some(tool) = s.extras.tools_registry.get(name) {
            yuantuan_core::tools::executor::register(&s.db_path, tool.as_ref())
                .await
                .map_err(error)?;
        }
    }
    for (name, description) in s.extras.skill_registry.describe_for_decision() {
        yuantuan_core::capabilities::register(
            &s.db_path,
            &name,
            "skill",
            &description,
            &json!({"type":"object"}),
        )
        .await
        .map_err(error)?;
    }
    let mut c = s.open_db().await.map_err(error)?;
    let rows=sqlx::query("SELECT c.*,d.definition_json,((SELECT COUNT(*) FROM capability_invocations i WHERE i.name=c.name AND i.kind=c.kind)+(SELECT COUNT(*) FROM task_invocations t WHERE c.kind='tool' AND t.tool_name=c.name)) AS calls FROM capability_catalog c LEFT JOIN tool_contracts d ON d.name=c.name AND c.kind='tool' ORDER BY kind,name").fetch_all(&mut *c).await.map_err(error)?;
    let items:Result<Vec<_>,sqlx::Error>=rows.iter().map(|r|{
        let name:String=r.try_get("name")?;let kind:String=r.try_get("kind")?;
        let available=if kind=="skill"{s.extras.skill_registry.get(&name).is_some()}else{s.extras.tools_registry.get(&name).is_some()};
        Ok(json!({"name":name,"kind":kind,"description":r.try_get::<String,_>("description")?,"enabled":r.try_get::<bool,_>("enabled")?,"available":available,"tool_contract":r.try_get::<Option<String>,_>("definition_json")?.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"scopes":serde_json::from_str::<Value>(&r.try_get::<String,_>("scopes")?).unwrap_or(json!([])),"calls":r.try_get::<i64,_>("calls")?}))
    }).collect();
    Ok(Json(json!({"capabilities":items.map_err(error)?})))
}
#[derive(Deserialize)]
pub struct Switch {
    name: String,
    kind: String,
    enabled: bool,
    #[serde(default)]
    scopes: Vec<String>,
}
pub async fn toggle(State(s): State<AppState>, Json(b): Json<Switch>) -> Response {
    if b.scopes.len() > 100
        || b.scopes
            .iter()
            .any(|v| !v.starts_with("group:") && !v.starts_with("private:"))
    {
        return Err(error("会话范围格式应为 group:群号 或 private:会话号"));
    }
    update_capability(&s, &b.name, &b.kind, Some(b.enabled), Some(b.scopes)).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPatch {
    name: String,
    kind: String,
    enabled: Option<bool>,
    scopes: Option<Vec<String>>,
}

pub async fn patch_capability(State(s): State<AppState>, Json(b): Json<CapabilityPatch>) -> Response {
    if b.enabled.is_none() && b.scopes.is_none() {
        return Err(error("请提供开关或会话范围"));
    }
    if b.scopes.as_ref().is_some_and(|scopes| {
        scopes.len() > 100
            || scopes.iter().any(|scope| {
                let id = scope
                    .strip_prefix("group:")
                    .or_else(|| scope.strip_prefix("private:"))
                    .unwrap_or("");
                id.is_empty()
                    || id.chars().count() > 128
                    || id.chars().any(|c| c.is_whitespace() || c.is_control() || c == ':')
            })
    }) {
        return Err(error("最多填写100个会话，格式为 group:群号 或 private:会话ID"));
    }
    update_capability(&s, &b.name, &b.kind, b.enabled, b.scopes).await
}

async fn update_capability(
    s: &AppState,
    name: &str,
    kind: &str,
    enabled: Option<bool>,
    scopes: Option<Vec<String>>,
) -> Response {
    let scopes = scopes
        .map(|values| serde_json::to_string(&values))
        .transpose()
        .map_err(error)?;
    let mut c = s.open_db().await.map_err(error)?;
    let row = sqlx::query(
        "UPDATE capability_catalog SET enabled=COALESCE(?3,enabled),scopes=COALESCE(?4,scopes)
         WHERE name=?1 AND kind=?2 RETURNING enabled,scopes",
    )
    .bind(name)
    .bind(kind)
    .bind(enabled)
    .bind(scopes)
    .fetch_optional(&mut *c)
    .await
    .map_err(error)?
    .ok_or_else(|| error("能力尚未注册"))?;
    let enabled: bool = row.try_get("enabled").map_err(error)?;
    let scopes: String = row.try_get("scopes").map_err(error)?;
    Ok(Json(json!({
        "ok": true,
        "enabled": enabled,
        "scopes": serde_json::from_str::<Value>(&scopes).unwrap_or(json!([])),
    })))
}
#[derive(Deserialize)]
pub struct Control {
    action: String,
    #[serde(default)]
    text: String,
}
pub async fn control(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Control>,
) -> Response {
    let mut c = s.open_db().await.map_err(error)?;
    let r=sqlx::query("SELECT t.created_by_pid,t.chat_id,r.chat_type FROM tasks t JOIN task_runtime r USING(task_id) WHERE t.task_id=?").bind(&id).fetch_one(&mut *c).await.map_err(error)?;
    yuantuan_core::agent::runtime::control(
        &s.db_path,
        &id,
        &r.try_get::<String, _>("created_by_pid").map_err(error)?,
        &r.try_get::<String, _>("chat_id").map_err(error)?,
        &r.try_get::<String, _>("chat_type").map_err(error)?,
        &b.action,
        &b.text,
    )
    .await
    .map_err(error)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn task(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let mut conn = s.open_db().await.map_err(error)?;
    let mut c = conn.transaction().await.map_err(error)?;
    let row = sqlx::query("SELECT * FROM task_runtime WHERE task_id=?")
        .bind(&id)
        .fetch_optional(&mut *c)
        .await
        .map_err(error)?;
    let Some(r) = row else {
        return Ok(Json(json!({"runtime":null})));
    };
    let brief = yuantuan_core::agent::brief::current(&mut c, &id)
        .await
        .map_err(error)?;
    let rows=sqlx::query("SELECT revision,event_kind,source_msg_id,created_at FROM task_brief_versions WHERE task_id=? ORDER BY revision DESC LIMIT 10").bind(&id).fetch_all(&mut *c).await.map_err(error)?;
    let versions:Result<Vec<_>,sqlx::Error>=rows.iter().map(|r|Ok(json!({"revision":r.try_get::<i64,_>("revision")?,"event_kind":r.try_get::<String,_>("event_kind")?,"source_msg_id":r.try_get::<Option<i64>,_>("source_msg_id")?,"created_at":r.try_get::<i64,_>("created_at")?}))).collect();
    let calls=sqlx::query("SELECT i.id,i.tool_name,i.state,i.revision,i.started_at,i.finished_at,m.call_id,m.protocol,m.status,m.error_json,m.elapsed_ms FROM task_invocations i LEFT JOIN tool_invocation_records m ON m.invocation_id=i.id WHERE i.task_id=? ORDER BY i.started_at,i.id LIMIT 100").bind(&id).fetch_all(&mut *c).await.map_err(error)?;
    let calls:Result<Vec<_>,sqlx::Error>=calls.iter().map(|r|Ok(json!({"id":r.try_get::<String,_>("id")?,"tool":r.try_get::<String,_>("tool_name")?,"call_id":r.try_get::<Option<String>,_>("call_id")?,"revision":r.try_get::<i64,_>("revision")?,"protocol":r.try_get::<Option<String>,_>("protocol")?,"status":r.try_get::<Option<String>,_>("status")?,"error":r.try_get::<Option<String>,_>("error_json")?.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"elapsed_ms":r.try_get::<Option<i64>,_>("elapsed_ms")?,"state":r.try_get::<String,_>("state")?,"started_at":r.try_get::<i64,_>("started_at")?,"finished_at":r.try_get::<Option<i64>,_>("finished_at")?}))).collect();
    Ok(Json(
        json!({"runtime":{"root_id":r.try_get::<String,_>("root_id").map_err(error)?,"parent_id":r.try_get::<Option<String>,_>("parent_id").map_err(error)?,"phase":r.try_get::<String,_>("phase").map_err(error)?,"revision":r.try_get::<i64,_>("revision").map_err(error)?,"budget_reserved":r.try_get::<i64,_>("spent_tokens").map_err(error)?,"budget_limit":r.try_get::<i64,_>("max_tokens").map_err(error)?,"delivery_state":r.try_get::<String,_>("delivery_state").map_err(error)?,"stop_reason":r.try_get::<Option<String>,_>("stop_reason").map_err(error)?,"final_text":r.try_get::<Option<String>,_>("final_text").map_err(error)?},"brief":brief,"brief_versions":versions.map_err(error)?,"invocations":calls.map_err(error)?}),
    ))
}
