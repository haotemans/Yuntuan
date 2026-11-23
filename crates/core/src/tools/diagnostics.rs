//! Read-only, caller-scoped diagnostics from existing durable runtime records.
use super::{Tool, ToolCancellation, ToolCtx, ToolEffect, ToolOutput, ToolPolicy};
use crate::{
    db::SqliteExt,
    skills::{Skill, SkillDef},
};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;

pub const NAME: &str = "diagnostics.recent";

#[derive(Clone)]
pub struct Diagnostics {
    db_path: PathBuf,
    sandbox_enabled: Option<bool>,
}

impl Diagnostics {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path, sandbox_enabled: None }
    }

    pub fn with_sandbox_enabled(mut self, enabled: bool) -> Self {
        self.sandbox_enabled = Some(enabled);
        self
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    minutes: i64,
    limit: i64,
    message_id: Option<i64>,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            minutes: 60,
            limit: 10,
            message_id: None,
        }
    }
}

/// Only fixed codes leave the runtime. Provider bodies and URLs may contain secrets.
pub(crate) fn error_code(error: &str) -> &'static str {
    let text = error.to_lowercase();
    if text.contains("401") || text.contains("unauthorized") {
        "authentication_rejected"
    } else if text.contains("403") || text.contains("permission_denied") {
        "permission_denied"
    } else if text.contains("429") || text.contains("rate limit") || text.contains("成本闸") {
        "rate_limited"
    } else if text.contains("timeout") || text.contains("timed out") || text.contains("超时") {
        "timeout"
    } else if text.contains("未配置") || text.contains("unavailable") {
        "unavailable"
    } else if text.contains("invalid_arguments") {
        "invalid_arguments"
    } else if text.contains("invalid_observation") || text.contains("schema") || text.contains("格式") {
        "invalid_output"
    } else if text.contains("时限") || text.contains("过期") || text.contains("时效") {
        "expired"
    } else {
        "failure_recorded"
    }
}

fn explanation(code: &str) -> &'static str {
    match code {
        "authentication_rejected" => "服务拒绝认证，请管理员检查凭据和服务权限",
        "permission_denied" => "权限检查未通过，请检查能力开关和会话授权",
        "rate_limited" => "遇到限流，请稍后再试并检查调用额度",
        "timeout" => "处理超时，需检查对应服务的响应情况",
        "unavailable" => "所需能力或模型未就绪，请检查配置与运行状态",
        "invalid_arguments" => "工具参数校验失败，请核对请求条件",
        "invalid_output" => "返回格式校验失败，请检查模型或接口兼容性",
        "expired" => "请求或回复已超过处理时限",
        "image_unseen" => "图片已收到，尚未触发识图，可引用该图并 @ 云团提问",
        "image_processing" => "已开始识图，记录尚未出现结果",
        "image_or_provider" => "识图失败，现有记录未细分图片下载与模型服务原因",
        "command_failed" => "Bash 命令已返回失败结果，请核对退出码及工作区中的实际产物",
        "unknown" => "执行结果待核实，请核对实际回执",
        _ => "已记录失败，现有记录未提供更具体的原因",
    }
}

#[async_trait]
impl Tool for Diagnostics {
    fn name(&self) -> &'static str {
        NAME
    }

    fn description(&self) -> &'static str {
        "查询云团自身最近的运行错误、识图状态与工具回执，仅查询当前会话中提问者自己的请求。解释报错前先读取实际记录。"
    }

    fn available_in_chat(&self, ctx: &ToolCtx) -> bool {
        matches!(ctx.chat_type.as_str(), "group" | "private")
            && !ctx.chat_id.is_empty()
            && !ctx.sender_pid.is_empty()
            && ctx.sender_pid != "self"
    }

    fn policy(&self) -> ToolPolicy {
        ToolPolicy {
            effect: ToolEffect::ReadOnly,
            timeout_ms: 10_000,
            cancellation: ToolCancellation::DropFuture,
            parallel_safe: true,
        }
    }

    fn parameters_schema(&self) -> Value {
        json!({"type":"object","additionalProperties":false,"properties":{
            "minutes":{"type":"integer","minimum":1,"maximum":1440,"description":"最近多少分钟，默认60"},
            "limit":{"type":"integer","minimum":1,"maximum":20,"description":"最多返回多少条，默认10"},
            "message_id":{"type":"integer","minimum":1,"description":"可选的本地消息编号，只能查询本人在当前会话的消息"}
        }})
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        ensure!(
            <Self as Tool>::available_in_chat(self, ctx),
            "诊断需要真实的会话与提问者身份"
        );
        crate::capabilities::check_tool(
            &self.db_path, NAME, ctx, &self.parameters_schema(), &args,
        ).await?;
        let query: Query = serde_json::from_value(args)?;
        let now = crate::window::now();
        let mut db = crate::db::connect(&self.db_path).await?;
        let flow = request_flow(&mut db, ctx, &query, now).await?;
        // Every source is joined to the caller's typed conversation and identity.
        // Raw descriptions are used only to map a fixed diagnostic code locally.
        let rows = sqlx::query(
            "WITH mine AS (
                SELECT * FROM messages WHERE chat_id=?1 AND chat_type=?2 AND sender_pid=?3
                AND ts>=?4 AND ts<=?5 AND (?6 IS NULL OR msg_id=?6)
            ), records AS (
                SELECT 'decision' AS stage, 'messages' AS source, msg_id AS source_id,
                    msg_id AS message_id, ts, decision_state AS state, decision_error AS detail
                FROM mine WHERE decision_state IN ('failed','uncertain') OR decision_error IS NOT NULL
                UNION ALL
                SELECT 'image','message_images',i.msg_id,i.msg_id,
                    CASE WHEN i.observed_at>0 THEN i.observed_at ELSE m.ts END,i.status,
                    CASE i.status WHEN 'pending' THEN 'image_unseen' WHEN 'processing' THEN 'image_processing'
                    ELSE COALESCE(s.value,'image_or_provider') END
                FROM message_images i JOIN mine m ON m.msg_id=i.msg_id
                LEFT JOIN state_kv s ON s.key='vision:failure:'||i.msg_id||':'||i.image_index
                WHERE i.status IN ('pending','processing','failed')
                UNION ALL
                SELECT 'skill','capability_invocations',i.id,i.anchor_id,i.started_at,i.state,NULL
                FROM capability_invocations i JOIN mine m ON m.msg_id=i.anchor_id
                WHERE i.chat_id=?1 AND i.chat_type=?2 AND i.actor=?3 AND i.state IN ('failed','unknown')
                UNION ALL
                SELECT 'tool','task_invocations',i.id,r.anchor_id,i.started_at,i.state,
                    CASE WHEN i.tool_name='sandbox_exec' AND json_extract(CASE WHEN json_valid(i.result) THEN i.result ELSE '{}' END,'$.data.success')=0
                        THEN CASE WHEN json_extract(i.result,'$.data.timed_out')=1 THEN 'timeout' ELSE 'command_failed' END
                        ELSE json_extract(CASE WHEN json_valid(x.error_json) THEN x.error_json ELSE '{}' END,'$.kind') END
                FROM task_invocations i JOIN tasks t ON t.task_id=i.task_id
                JOIN task_runtime r ON r.task_id=t.task_id
                LEFT JOIN tool_invocation_records x ON x.invocation_id=i.id
                WHERE t.chat_id=?1 AND r.chat_type=?2 AND t.created_by_pid=?3 AND i.started_at>=?4
                    AND i.started_at<=?5 AND (?6 IS NULL OR r.anchor_id=?6)
                    AND (i.state IN ('failed','unknown') OR x.status IN ('failed','unknown','rejected','cancelled')
                        OR (i.tool_name='sandbox_exec' AND json_extract(CASE WHEN json_valid(i.result) THEN i.result ELSE '{}' END,'$.data.success')=0))
                UNION ALL
                SELECT json_extract(e.payload,'$.stage'),'events',e.id,m.msg_id,e.ts,'failed',
                    json_extract(e.payload,'$.code')
                FROM events e JOIN mine m ON m.msg_id=json_extract(e.payload,'$.source_msg_id')
                WHERE e.kind='RuntimeFailure' AND json_extract(e.payload,'$.chat_id')=?1
                    AND json_extract(e.payload,'$.chat_type')=?2 AND e.ts>=?4 AND e.ts<=?5
            ) SELECT * FROM records ORDER BY ts DESC, source, source_id DESC LIMIT ?7",
        )
        .bind(&ctx.chat_id)
        .bind(&ctx.chat_type)
        .bind(&ctx.sender_pid)
        .bind(now - query.minutes * 60)
        .bind(now)
        .bind(query.message_id)
        .bind(query.limit + 1)
        .fetch_all(&mut *db)
        .await?;
        let has_more = rows.len() > query.limit as usize;
        let mut entries = Vec::new();
        let mut lines = vec![format!("最近{}分钟，当前会话中你的请求诊断", query.minutes)];
        if let Some(flow) = &flow {
            lines.push(format!("消息{}的流程", flow["message_id"]));
            for stage in flow["stages"].as_array().into_iter().flatten() {
                lines.push(format!("{} · {}", stage["label"].as_str().unwrap_or_default(),
                    stage["description"].as_str().unwrap_or_default()));
            }
        }
        for row in rows.into_iter().take(query.limit as usize) {
            let detail = row.try_get::<Option<String>, _>("detail")?.unwrap_or_default();
            let state: String = row.try_get("state")?;
            let code = match detail.as_str() {
                "image_unseen" | "image_processing" | "image_or_provider" | "timeout" => detail.as_str(),
                "authentication_rejected" | "permission_denied" | "rate_limited" | "unavailable"
                | "invalid_arguments" | "invalid_output" | "expired" | "failure_recorded" | "command_failed" => detail.as_str(),
                _ if state == "unknown" || state == "uncertain" => "unknown",
                _ => error_code(&detail),
            };
            let stage: String = row.try_get("stage")?;
            let ts: i64 = row.try_get("ts")?;
            let message_id: Option<i64> = row.try_get("message_id")?;
            let source_id: Value = match row.try_get::<i64, _>("source_id") {
                Ok(id) => json!(id),
                Err(_) => json!(row.try_get::<String, _>("source_id")?),
            };
            let label = match stage.as_str() {
                "decision" => "参与判断",
                "image" => "图片识别",
                "tool" => "工具调用",
                "skill" => "技能执行",
                "chat" => "回复生成",
                "delivery" => "消息发送",
                _ => "运行记录",
            };
            let reason = explanation(code);
            let message_label = message_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "未关联".into());
            lines.push(format!(
                "{}分钟前 · {} · 消息{} · {}",
                (now - ts).max(0) / 60, label, message_label, reason
            ));
            entries.push(json!({"stage":stage,"source":row.try_get::<String,_>("source")?,
                "source_id":source_id,"message_id":message_id,"ts":ts,"state":state,"code":code,"explanation":reason}));
        }
        if entries.is_empty() {
            lines.push("这段时间内没有可查询的失败或未处理图片记录。".into());
        }
        if has_more {
            lines.push("还有更早的记录，可缩短时间范围或指定消息编号。".into());
        }
        let (bash_status, bash_description) = match self.sandbox_enabled {
            Some(true) => ("enabled_at_startup", "Bash 已启用，启动时沙箱检查通过，可由 Agent 任务调用"),
            Some(false) => ("disabled", "Bash 未启用，当前进程未注册 sandbox_exec"),
            None => ("not_reported", "本次运行未提供 Bash 配置状态"),
        };
        lines.push(bash_description.into());
        lines.push("依据来自已保存的运行记录，当前服务状态和未记录的故障仍需进一步核实。".into());
        let rendered = lines.join("\n");
        Ok(ToolOutput {
            summary: format!("读取到{}条诊断记录", entries.len()), artifacts: vec![],
            data: json!({"entries":entries,"flow":flow,"bash":{"status":bash_status,"description":bash_description},
                "has_more":has_more,"from_ts":now-query.minutes*60,
                "until_ts":now,"scope":"current_actor_in_current_conversation","rendered":rendered}),
        })
    }
}

async fn request_flow(
    db: &mut sqlx::SqliteConnection,
    ctx: &ToolCtx,
    query: &Query,
    now: i64,
) -> Result<Option<Value>> {
    let Some(id) = query.message_id else { return Ok(None) };
    let message = sqlx::query(
        "SELECT msg_id,ts,decision_state,decision_attempts FROM messages
         WHERE msg_id=?1 AND chat_id=?2 AND chat_type=?3 AND sender_pid=?4
         AND ts BETWEEN ?5 AND ?6",
    )
    .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).bind(&ctx.sender_pid)
    .bind(now - query.minutes * 60).bind(now)
    .fetch_optional(&mut *db).await?;
    let Some(message) = message else { return Ok(None) };
    let state: String = message.try_get("decision_state")?;
    let description = match state.as_str() {
        "pending" => "等待处理",
        "claimed" => "已领取，等待执行",
        "deciding" => "正在判断",
        "executing" => "正在处理回复或执行动作",
        "done" => "本条消息处理已结束，具体动作以调用和发送回执为准",
        "failed" => "已停止自动处理，可查看下面的原因记录",
        "uncertain" => "执行中断，结果待核实",
        _ => "状态需核实",
    };
    let mut stages = vec![
        json!({"stage":"ingest","label":"消息接收","state":"stored","description":"原消息已保存",
            "ts":message.try_get::<i64,_>("ts")?}),
        json!({"stage":"decision","label":"消息处理","state":state,"description":description,
            "attempts":message.try_get::<i64,_>("decision_attempts")?}),
    ];
    let images = sqlx::query("SELECT image_index,status,observed_at FROM message_images WHERE msg_id=?1 ORDER BY image_index LIMIT 3")
        .bind(id).fetch_all(&mut *db).await?;
    let image_records: Vec<Value> = images.iter().map(|row| -> Result<Value> {
        Ok(json!({"image_index":row.try_get::<i64,_>("image_index")?,
            "state":row.try_get::<String,_>("status")?,"observed_at":row.try_get::<i64,_>("observed_at")?}))
    }).collect::<Result<_>>()?;
    let image_description = if image_records.is_empty() {
        "本条消息无保存的附图记录，引用图片的状态按其原消息查询".into()
    } else {
        image_records.iter().map(|image| {
            let state = match image["state"].as_str() {
                Some("pending") => "尚未触发",
                Some("processing") => "等待识图结果",
                Some("ready") => "识图结果已保存",
                Some("failed") => "识图失败",
                _ => "状态需核实",
            };
            format!("第{}张图{}", image["image_index"].as_i64().unwrap_or(0)+1, state)
        }).collect::<Vec<_>>().join("，")
    };
    stages.push(json!({"stage":"image","label":"本条消息附图","state":"recorded",
        "description":image_description,
        "records":image_records}));
    let skills = sqlx::query("SELECT state,started_at,finished_at FROM capability_invocations WHERE anchor_id=?1 AND chat_id=?2 AND chat_type=?3 AND actor=?4 ORDER BY id DESC LIMIT 10")
        .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).bind(&ctx.sender_pid)
        .fetch_all(&mut *db).await?;
    let skill_records: Vec<Value> = skills.iter().map(|row| -> Result<Value> {
        Ok(json!({"state":row.try_get::<String,_>("state")?,"started_at":row.try_get::<i64,_>("started_at")?,
            "finished_at":row.try_get::<Option<i64>,_>("finished_at")?}))
    }).collect::<Result<_>>()?;
    let tasks = sqlx::query("SELECT r.phase,r.delivery_state,r.revision,r.updated_at FROM task_runtime r JOIN tasks t USING(task_id) WHERE r.anchor_id=?1 AND t.chat_id=?2 AND r.chat_type=?3 AND t.created_by_pid=?4 ORDER BY r.updated_at DESC LIMIT 10")
        .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).bind(&ctx.sender_pid)
        .fetch_all(&mut *db).await?;
    let task_records: Vec<Value> = tasks.iter().map(|row| -> Result<Value> {
        Ok(json!({"state":row.try_get::<String,_>("phase")?,"delivery_state":row.try_get::<String,_>("delivery_state")?,
            "revision":row.try_get::<i64,_>("revision")?,"updated_at":row.try_get::<i64,_>("updated_at")?}))
    }).collect::<Result<_>>()?;
    let execution_description = if skill_records.is_empty() && task_records.is_empty() {
        "当前未查到该原消息关联的技能或任务记录".into()
    } else {
        let states = skill_records.iter().chain(&task_records).map(|record| {
            match record["state"].as_str() {
                Some("succeeded" | "finished") => "执行已结束",
                Some("failed") => "执行失败",
                Some("unknown") => "执行结果待核实",
                Some("waiting_input") => "等待补充信息",
                Some("waiting_children") => "等待子任务",
                Some("started" | "running" | "executing") => "执行中",
                Some("queued") => "等待执行",
                Some("paused") => "已暂停",
                Some("cancelled") => "已取消",
                _ => "状态需核实",
            }
        }).collect::<Vec<_>>().join("，");
        format!("关联{}次技能调用、{}个任务，{}", skill_records.len(), task_records.len(), states)
    };
    stages.push(json!({"stage":"execution","label":"技能与任务","state":"recorded",
        "description":execution_description,"skills":skill_records,"tasks":task_records}));
    let tools = sqlx::query(
        "SELECT i.tool_name,COALESCE(x.status,i.state) AS state,
         CASE WHEN i.tool_name='sandbox_exec' THEN json_extract(CASE WHEN json_valid(i.result) THEN i.result ELSE '{}' END,'$.data.exit_code') END AS exit_code,
         CASE WHEN i.tool_name='sandbox_exec' THEN json_extract(CASE WHEN json_valid(i.result) THEN i.result ELSE '{}' END,'$.data.timed_out') END AS timed_out
         FROM task_invocations i JOIN tasks t ON t.task_id=i.task_id JOIN task_runtime r ON r.task_id=t.task_id
         LEFT JOIN tool_invocation_records x ON x.invocation_id=i.id
         WHERE r.anchor_id=?1 AND t.chat_id=?2 AND r.chat_type=?3 AND t.created_by_pid=?4
         ORDER BY i.started_at DESC,i.id DESC LIMIT 10",
    )
    .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).bind(&ctx.sender_pid)
    .fetch_all(&mut *db).await?;
    let tool_records: Vec<Value> = tools.iter().map(|row| -> Result<Value> {
        Ok(json!({"kind":if row.try_get::<String,_>("tool_name")? == "sandbox_exec" {"bash"}else{"tool"},
            "state":row.try_get::<String,_>("state")?,"exit_code":row.try_get::<Option<i64>,_>("exit_code")?,
            "timed_out":row.try_get::<Option<bool>,_>("timed_out")?}))
    }).collect::<Result<_>>()?;
    let tool_description = if tool_records.is_empty() {
        "尚无该原消息关联的任务工具调用记录".into()
    } else {
        tool_records.iter().map(|record| {
            if record["kind"] == "bash" {
                let exit_code = record["exit_code"].as_i64()
                    .map(|code| code.to_string()).unwrap_or_else(|| "尚无记录".into());
                let timeout = match record["timed_out"].as_bool() {
                    Some(true) => "已超时", Some(false) => "未超时", None => "超时状态尚无记录",
                };
                format!("Bash 退出码{}，{}", exit_code, timeout)
            } else {
                "工具调用已有保存记录，可在任务回放核对实际结果".into()
            }
        }).collect::<Vec<_>>().join("，")
    };
    stages.push(json!({"stage":"tool","label":"工具与 Bash","state":"recorded",
        "description":tool_description,"records":tool_records}));
    let sent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE reply_anchor_id=?1 AND chat_id=?2 AND chat_type=?3 AND sender_pid='self'")
        .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).fetch_one(&mut *db).await?;
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE reply_anchor_id=?1 AND chat_id=?2 AND chat_type=?3 AND sender_pid='self' AND external_msg_id IS NOT NULL")
        .bind(id).bind(&ctx.chat_id).bind(&ctx.chat_type).fetch_one(&mut *db).await?;
    let (delivery_state, delivery_description) = if receipts > 0 {
        ("receipt_recorded", "已保存回复及平台消息编号")
    } else if sent > 0 {
        ("reply_recorded", "已保存发送后的回复记录，平台数字消息编号未记录")
    } else {
        ("unconfirmed", "尚无该原消息关联的发送回执")
    };
    stages.push(json!({"stage":"delivery","label":"消息交付","state":delivery_state,
        "description":delivery_description,"sent_replies":sent,"numeric_receipts":receipts}));
    Ok(Some(json!({"message_id":id,"stages":stages})))
}

#[async_trait]
impl Skill for Diagnostics {
    fn def(&self) -> SkillDef {
        SkillDef {
            name: NAME,
            description: <Self as Tool>::description(self),
            prompt_template: String::new(),
            tools: vec![NAME],
            role_hint: None,
        }
    }
    fn is_information_lookup(&self) -> bool {
        true
    }
    fn commands(&self) -> &'static [&'static str] {
        &["/自检", "/diagnose"]
    }
    fn accepts_empty_command(&self) -> bool {
        true
    }
    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let text = slots["user_text"].as_str().unwrap_or("").trim();
        let message_id = if slots["explicit_command"] == true && !text.is_empty() {
            Some(text.parse::<i64>().map_err(|_| anyhow::anyhow!("用法为 /自检 或 /自检 消息编号"))?)
        } else {
            slots["message_id"].as_i64()
        };
        let args = match message_id {
            Some(id) => json!({"message_id":id}),
            None => json!({}),
        };
        <Self as Tool>::call(self, ctx, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::params;

    #[tokio::test]
    async fn reports_real_sources_without_exposing_other_people_or_raw_errors() {
        let path = std::env::temp_dir().join(format!("yt-diagnostics-{}.db", rand::random::<u64>()));
        let mut db = crate::db::connect(&path).await.unwrap();
        crate::db::migrate(&mut db).await.unwrap();
        db.execute_batch(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_1',1,1),('p_2',1,1);
             INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,decision_state,decision_error) VALUES
             (1,'42','group','p_1','own',unixepoch(),'failed','HTTP 401 Bearer secret-provider-key'),
             (2,'42','group','p_2','other member',unixepoch(),'failed','private error'),
             (3,'42','private','p_1','same id private',unixepoch(),'failed','private error'),
             (4,'43','group','p_1','other group',unixepoch(),'failed','private error'),
             (5,'42','group','p_1','too old',unixepoch()-90000,'failed','old error');
             INSERT INTO message_images(msg_id,image_index,url,status) VALUES
             (1,0,'https://signed.test/private?token=secret','pending'),
             (2,0,'https://signed.test/other','failed');
             INSERT INTO capability_invocations(name,kind,chat_id,chat_type,actor,anchor_id,state,started_at)
             VALUES('plugin.example','skill','42','group','p_1',1,'unknown',unixepoch());
             INSERT INTO events(kind,payload,ts) VALUES('RuntimeFailure',
             '{\"chat_id\":\"42\",\"chat_type\":\"group\",\"source_msg_id\":1,\"stage\":\"delivery\",\"code\":\"timeout\"}',unixepoch());
             INSERT INTO tasks(task_id,goal,state,budget_max_calls,created_by_pid,chat_id,created_at)
             VALUES('t','private goal','failed',1,'p_1','42',unixepoch());
             INSERT INTO task_runtime(task_id,root_id,chat_type,anchor_id,deadline,max_tokens,updated_at)
             VALUES('t','t','group',1,unixepoch()+60,10000,unixepoch());
             INSERT INTO task_invocations(id,task_id,revision,tool_name,args,state,started_at)
             VALUES('call','t',1,'private-tool','{\"key\":\"secret-argument\"}','failed',unixepoch());
             INSERT INTO task_invocations(id,task_id,revision,tool_name,args,state,started_at,result)
             VALUES('bash-call','t',1,'sandbox_exec','{\"command\":\"private-command\"}','succeeded',unixepoch(),
                '{\"data\":{\"success\":false,\"exit_code\":7,\"timed_out\":false,\"stderr\":\"private-output\"}}');",
        ).await.unwrap();
        let ctx = ToolCtx { task_id: None, chat_id: "42".into(), chat_type: "group".into(),
            sender_pid: "p_1".into(), locale: None };
        let tool = Diagnostics::new(path.clone()).with_sandbox_enabled(false);
        let output = tool.call(&ctx, json!({})).await.unwrap();
        let entries = output.data["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().all(|item| item["message_id"] == 1));
        assert!(entries.iter().any(|item| item["code"] == "authentication_rejected"));
        assert!(entries.iter().any(|item| item["code"] == "image_unseen" && item["state"] == "pending"));
        assert!(entries.iter().any(|item| item["stage"] == "delivery" && item["code"] == "timeout"));
        assert!(entries.iter().any(|item| item["code"] == "command_failed"));
        assert_eq!(output.data["bash"]["status"], "disabled");
        let text = serde_json::to_string(&output).unwrap();
        for private in ["secret", "private error", "signed.test", "private goal", "private-tool", "old error", "private-command", "private-output"] {
            assert!(!text.contains(private), "leaked {private}");
        }
        let limited = tool.call(&ctx, json!({"limit":1})).await.unwrap();
        assert_eq!(limited.data["entries"].as_array().unwrap().len(), 1);
        assert_eq!(limited.data["has_more"], true);
        let foreign = tool.call(&ctx, json!({"message_id":2})).await.unwrap();
        assert_eq!(foreign.data["entries"], json!([]));
        assert!(foreign.data["flow"].is_null());
        let focused = tool.call(&ctx, json!({"message_id":1})).await.unwrap();
        let stages = focused.data["flow"]["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0]["state"], "stored");
        assert_eq!(stages[1]["state"], "failed");
        assert_eq!(stages[2]["records"][0]["state"], "pending");
        assert_eq!(stages[5]["state"], "unconfirmed");
        assert!(stages[4]["records"].as_array().unwrap().iter().any(|r| r["kind"]=="bash" && r["exit_code"]==7));
        let focused_text = serde_json::to_string(&focused).unwrap();
        assert!(!focused_text.contains("private-command"));
        assert!(!focused_text.contains("private-output"));
        db.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('self',1,1);
            INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,reply_anchor_id,external_msg_id)
            VALUES('42','group','self','reply',unixepoch(),1,77);").await.unwrap();
        let delivered = tool.call(&ctx,json!({"message_id":1})).await.unwrap();
        assert_eq!(delivered.data["flow"]["stages"][5]["state"], "receipt_recorded");
        assert_eq!(delivered.data["flow"]["stages"][5]["numeric_receipts"], 1);
        for args in [json!({"minutes":0}), json!({"limit":100}), json!({"chat_id":"43"}), json!({"message_id":-1})] {
            assert!(tool.call(&ctx, args).await.is_err());
        }
        crate::tools::executor::register(&path, &tool).await.unwrap();
        db.execute("UPDATE capability_catalog SET enabled=0 WHERE name=?1 AND kind='tool'",params![NAME]).await.unwrap();
        assert!(tool.call(&ctx, json!({})).await.is_err());
        assert!(Skill::invoke(&tool, &ctx, json!({})).await.is_err());
    }
}
