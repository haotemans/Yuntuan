use crate::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use yuantuan_core::management::{Action, Caller, Grant, Management};
type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn service(s: &AppState) -> Result<&std::sync::Arc<Management>, (StatusCode, Json<Value>)> {
    s.extras
        .management
        .as_ref()
        .ok_or_else(|| error("机器人管理服务未装配"))
}
pub async fn status(State(s): State<AppState>) -> Response {
    let m = service(&s)?;
    let mut policy = m.policy();
    policy["configured_owner_qq"] =
        json!(crate::config_api::read_toml_as_json(&s.extras.config_path)
            .and_then(|v| v["security"]["super_admin_qq"].as_u64())
            .filter(|v| *v > 0)
            .map(|v| v.to_string()));
    Ok(Json(
        json!({"policy":policy,"plugins":m.status(&Caller::Webui).map_err(error)?,"history":m.history().await.map_err(error)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    grants: Vec<Grant>,
}
pub async fn permissions(State(s): State<AppState>, Json(b): Json<Permissions>) -> Response {
    service(&s)?.set_grants(b.grants).await.map_err(error)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn apply(State(s): State<AppState>, Json(b): Json<Action>) -> Response {
    Ok(Json(
        service(&s)?.apply(&Caller::Webui, b).await.map_err(error)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    qq: String,
}
pub async fn owner(State(s): State<AppState>, Json(b): Json<Owner>) -> Response {
    let qq =
        b.qq.parse::<u64>()
            .ok()
            .filter(|v| *v > 0 && v.to_string() == b.qq)
            .ok_or_else(|| error("主人 QQ 必须是完整数字 ID"))?;
    let mut config = crate::config_api::read_toml_as_json(&s.extras.config_path)
        .ok_or_else(|| error("无法读取当前配置，未写入"))?;
    if config.get("security").is_none() {
        config["security"] = json!({})
    }
    config["security"]["super_admin_qq"] = json!(qq);
    crate::config_api::post_config(
        State(s),
        Json(crate::config_api::WriteBody {
            config: Some(config),
            providers: None,
        }),
    )
    .await
}
