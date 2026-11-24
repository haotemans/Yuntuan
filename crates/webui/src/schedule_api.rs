//! Authenticated WebUI endpoints for scheduled tools and the Quark share watcher.
use crate::AppState;
use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use yuantuan_core::scheduler;

type ApiError=(StatusCode,Json<Value>);
fn err(message:impl ToString)->ApiError {
    (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":message.to_string()})))
}
pub async fn list(State(state):State<AppState>)->Result<Json<Value>,ApiError> {
    let watches=scheduler::list_quark(&state.db_path).await.map_err(err)?;
    Ok(Json(json!({"ok":true,"watches":watches,"time_zone":"Asia/Shanghai","daily_check_time":"09:00"})))
}
#[derive(Deserialize)]
pub struct AddBody {
    url:String,
    #[serde(default)]
    title:String,
}
pub async fn add(State(state):State<AppState>,Json(body):Json<AddBody>)->Result<Json<Value>,ApiError> {
    let id=scheduler::register_quark(&state.db_path,&state.extras.tools_registry,&body.url,&body.title)
        .await.map_err(err)?;
    Ok(Json(json!({"ok":true,"id":id})))
}
#[derive(Deserialize)]
pub struct ToggleBody { enabled:bool }
pub async fn toggle(State(state):State<AppState>,Path(id):Path<i64>,Json(body):Json<ToggleBody>)->Result<Json<Value>,ApiError> {
    let applied=scheduler::toggle(&state.db_path,id,body.enabled).await.map_err(err)?;
    if !applied {return Err((StatusCode::NOT_FOUND,Json(json!({"ok":false,"error":"没有找到对应的分享监控任务"}))))}
    Ok(Json(json!({"ok":true})))
}
pub async fn remove(State(state):State<AppState>,Path(id):Path<i64>)->Result<Json<Value>,ApiError> {
    let removed=scheduler::remove(&state.db_path,id).await.map_err(err)?;
    if !removed {return Err((StatusCode::NOT_FOUND,Json(json!({"ok":false,"error":"没有找到对应的分享监控任务"}))))}
    Ok(Json(json!({"ok":true})))
}
pub async fn check_now(State(state):State<AppState>,Path(id):Path<i64>)->Result<Json<Value>,ApiError> {
    scheduler::check_now(&state.db_path,&state.extras.tools_registry,id).await.map_err(err)?;
    Ok(Json(json!({"ok":true,"message":"已记录最新检查结果"})))
}

pub async fn tasks(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let tasks = scheduler::list_tasks(&state.db_path, None).await.map_err(err)?;
    let options = scheduler::task_options(&state.db_path, &state.extras.tools_registry).await.map_err(err)?;
    Ok(Json(json!({"tasks":tasks,"options":options,"time_zone":"Asia/Shanghai"})))
}

#[derive(Deserialize)]
pub struct TaskBody {
    tool_name: String,
    daily_time: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    title: String,
}

pub async fn add_task(State(state): State<AppState>, Json(body): Json<TaskBody>) -> Result<Json<Value>, ApiError> {
    let id = scheduler::register_task(&state.db_path, &state.extras.tools_registry,
        &body.tool_name, &body.input, &body.title, &body.daily_time).await.map_err(err)?;
    Ok(Json(json!({"ok":true,"id":id})))
}

#[derive(Deserialize)]
pub struct TimeBody { daily_time: String }

pub async fn set_time(State(state): State<AppState>, Path(id): Path<i64>, Json(body): Json<TimeBody>) -> Result<Json<Value>, ApiError> {
    if !scheduler::set_task_time(&state.db_path, id, &body.daily_time).await.map_err(err)? {
        return Err(err("任务正在执行或已不存在"));
    }
    Ok(Json(json!({"ok":true})))
}

pub async fn toggle_task(State(state): State<AppState>, Path(id): Path<i64>, Json(body): Json<ToggleBody>) -> Result<Json<Value>, ApiError> {
    if !scheduler::toggle_task(&state.db_path, id, body.enabled, None).await.map_err(err)? {
        return Err(err("没有找到对应的定时任务"));
    }
    Ok(Json(json!({"ok":true})))
}

pub async fn remove_task(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    if !scheduler::remove_task(&state.db_path, id, None).await.map_err(err)? {
        return Err(err("任务正在执行或已不存在"));
    }
    Ok(Json(json!({"ok":true})))
}

pub async fn run_now(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    scheduler::run_now(&state.db_path, &state.extras.tools_registry, id, None).await.map_err(err)?;
    Ok(Json(json!({"ok":true})))
}
