use crate::AppState;
use axum::{extract::{Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Default, Deserialize)]
pub struct LogQuery {
    level: Option<String>,
    target: Option<String>,
    search: Option<String>,
    from_ts: Option<u64>,
    until_ts: Option<u64>,
    before_id: Option<u64>,
    after_id: Option<u64>,
    limit: Option<usize>,
}

pub async fn logs(State(state): State<AppState>, Query(q): Query<LogQuery>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if matches!((q.from_ts,q.until_ts),(Some(a),Some(b)) if a>=b)
        || q.search.as_ref().is_some_and(|s|s.len()>256)
        || q.target.as_ref().is_some_and(|s|s.len()>160)
        || q.level.as_ref().is_some_and(|s|!matches!(s.as_str(),"TRACE"|"DEBUG"|"INFO"|"WARN"|"ERROR")) {
        return Err((StatusCode::BAD_REQUEST,Json(json!({"error":"日志筛选条件无效"}))));
    }
    let limit = q.limit.unwrap_or(100).clamp(1,200);
    let mut entries = state.extras.runtime_logs.entries();
    let latest_id = entries.last().map(|entry|entry.id);
    let oldest_id = entries.first().map(|entry|entry.id);
    let targets: std::collections::BTreeSet<_> = entries.iter().map(|entry|entry.target.clone()).collect();
    let search = q.search.as_deref().unwrap_or("").to_lowercase();
    entries.retain(|entry| {
        q.level.as_ref().is_none_or(|level|entry.level==*level)
            && q.target.as_ref().is_none_or(|target|entry.target.contains(target))
            && q.from_ts.is_none_or(|ts|entry.ts>=ts)
            && q.until_ts.is_none_or(|ts|entry.ts<ts)
            && q.before_id.is_none_or(|id|entry.id<id)
            && q.after_id.is_none_or(|id|entry.id>id)
            && (search.is_empty() || entry.message.to_lowercase().contains(&search)
                || entry.fields.values().any(|value|value.to_lowercase().contains(&search)))
    });
    // Incremental reads return the oldest unseen prefix to avoid losing busy intervals.
    if q.after_id.is_none() { entries.reverse(); }
    let has_more = entries.len()>limit;
    entries.truncate(limit);
    Ok(Json(json!({"entries":entries,"has_more":has_more,"latest_id":latest_id,"oldest_id":oldest_id,
        "gap":matches!((q.after_id,oldest_id),(Some(after),Some(oldest)) if after<oldest),
        "targets":targets,"storage":state.extras.runtime_logs.status()})))
}

pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let process = tokio::task::spawn_blocking(process_info).await.unwrap_or_else(|_|json!({"unavailable":true}));
    let uptime = process["run_time_secs"].as_u64().unwrap_or_else(||state.started.elapsed().as_secs());
    Json(json!({"process":process,"uptime_secs":uptime,
        "components":state.extras.supervisor.snapshot(),"total_restarts":state.extras.supervisor.total_restarts(),
        "platforms":(state.extras.platform_status)(),"storage":state.extras.runtime_logs.status(),
        "bash_registered":state.extras.tools_registry.get("sandbox_exec").is_some()}))
}

fn process_info() -> Value {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    static SYSTEM: std::sync::Mutex<Option<System>> = std::sync::Mutex::new(None);
    let mut guard = SYSTEM.lock().unwrap_or_else(|p|p.into_inner());
    let system = guard.get_or_insert_with(System::new);
    system.refresh_processes(ProcessesToUpdate::All,true);
    let pid = Pid::from_u32(std::process::id());
    let own = system.process(pid);
    let mut parents = vec![pid];
    let mut children = Vec::new();
    for _ in 0..4 {
        let next: Vec<_> = system.processes().iter().filter(|(id,p)| {
            !parents.contains(id) && p.parent().is_some_and(|parent|parents.contains(&parent))
        }).map(|(id,_)|*id).collect();
        for id in &next {
            if let Some(process) = system.process(*id) {
                children.push(json!({"pid":id.as_u32(),"name":process.name().to_string_lossy(),
                    "memory_bytes":process.memory(),"cpu_percent":process.cpu_usage(),"run_time_secs":process.run_time()}));
            }
        }
        if next.is_empty() || children.len()>=64 { break; }
        parents.extend(next);
    }
    children.truncate(64);
    json!({"pid":pid.as_u32(),"memory_bytes":own.map(|p|p.memory()),"cpu_percent":own.map(|p|p.cpu_usage()),
        "run_time_secs":own.map(|p|p.run_time()),"children":children})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn filtering_and_incremental_cursors_preserve_log_records() {
        let root=std::env::temp_dir().join(format!("yt-logs-api-{}",rand::random::<u64>()));
        let state=AppState::new(root.join("db.sqlite"),crate::Extras::for_test(root.join("config.toml"),root.join("providers.toml")));
        for level in ["INFO","ERROR","ERROR"] {
            state.extras.runtime_logs.record(level,"module","event",None,None,Default::default());
        }
        let Json(first)=logs(State(state.clone()),Query(LogQuery {limit:Some(1),level:Some("ERROR".into()),..Default::default()})).await.unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(),1);
        assert_eq!(first["has_more"],true);
        let before=first["entries"][0]["id"].as_u64().unwrap();
        let Json(older)=logs(State(state.clone()),Query(LogQuery {before_id:Some(before),level:Some("ERROR".into()),..Default::default()})).await.unwrap();
        assert_eq!(older["entries"].as_array().unwrap().len(),1);
        let after=older["entries"][0]["id"].as_u64().unwrap();
        let Json(newer)=logs(State(state.clone()),Query(LogQuery {after_id:Some(after),..Default::default()})).await.unwrap();
        assert_eq!(newer["entries"][0]["id"],before);
        assert!(logs(State(state),Query(LogQuery {from_ts:Some(2),until_ts:Some(1),..Default::default()})).await.is_err());
    }
}
