//! One guarded task-tool boundary for compiled plugins, MCP adapters and model protocols.
use super::*;
use crate::{
    agent::runtime::{self, Checkpoint},
    db::{params, SqliteExt},
};
use anyhow::{ensure, Result};
use futures_util::FutureExt;
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn effect_allowed(ctx: &ToolCtx, tool: &dyn Tool) -> bool {
    !(ctx.chat_type == "private" && ctx.chat_id == "scheduled" && ctx.task_id.is_some())
        || matches!(tool.policy().effect, ToolEffect::ReadOnly | ToolEffect::WorkspaceWrite)
}

pub async fn register(path: &Path, tool: &dyn Tool) -> Result<ToolDefinition> {
    let definition = ToolDefinition::of(tool);
    ensure!(
        definition.input_schema.is_object(),
        "工具输入Schema必须是对象"
    );
    crate::capabilities::register(
        path,
        tool.name(),
        "tool",
        tool.description(),
        &definition.input_schema,
    )
    .await?;
    let mut c = crate::db::connect(path).await?;
    c.execute("INSERT INTO tool_contracts(name,definition_json,updated_at) VALUES(?1,?2,unixepoch()) ON CONFLICT(name) DO UPDATE SET definition_json=excluded.definition_json,updated_at=excluded.updated_at WHERE definition_json!=excluded.definition_json",params![definition.name,serde_json::to_string(&definition)?]).await?;
    Ok(definition)
}

pub async fn catalog(
    path: &Path,
    registry: &Registry,
    ctx: &ToolCtx,
    allowed: &[String],
) -> Result<Vec<ToolDefinition>> {
    let mut names = registry.names();
    names.sort();
    let mut out = vec![];
    for name in names {
        if !allowed.is_empty() && !allowed.iter().any(|s| s == name) {
            continue;
        }
        if let Some(tool) = registry.get(name) {
            if tool.available_in_chat(ctx) && effect_allowed(ctx, tool.as_ref())
                && crate::capabilities::allowed(path, name, "tool", ctx).await?
            {
                out.push(register(path, tool.as_ref()).await?);
            }
        }
    }
    Ok(out)
}

pub struct Execution {
    pub call: ToolCall,
    pub protocol: ToolProtocol,
    pub result: ToolResult,
    pub audit_details: Option<Value>,
}
impl Execution {
    /// Model projection is bounded; the complete output is retained by commit().
    pub fn history_record(&self) -> Value {
        let mut result = serde_json::to_value(&self.result).unwrap();
        if let Some(output) = result.get_mut("output").filter(|v| v.is_object()) {
            if output["data"].to_string().len() > 16_000 {
                let preview = output["data"]
                    .to_string()
                    .chars()
                    .take(4000)
                    .collect::<String>();
                output["data"] =
                    json!({"truncated":true,"preview":preview,"receipt_id":self.result.receipt_id});
            }
            if let Some(s) = output["summary"]
                .as_str()
                .filter(|s| s.chars().count() > 2000)
            {
                output["summary"] = json!(s.chars().take(2000).collect::<String>());
                output["summary_truncated"] = json!(true);
            }
        }
        json!({"role":"tool_result","call":self.call,"protocol":self.protocol,"result":result})
    }
}
fn rejected(
    call: &ToolCall,
    ctx: &ToolCtx,
    s: &Checkpoint,
    kind: ToolErrorKind,
    message: impl Into<String>,
) -> Execution {
    Execution {
        call: call.clone(),
        protocol: ToolProtocol::Json,
        result: ToolResult {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            task_id: ctx.task_id.clone().unwrap_or_default(),
            revision: s.revision,
            receipt_id: None,
            status: ToolStatus::Rejected,
            output: None,
            error: Some(ToolError::new(kind, message)),
            retryable: kind == ToolErrorKind::InvalidArguments,
            external_action_started: false,
            elapsed_ms: 0,
        },
        audit_details: None,
    }
}

pub async fn execute(
    path: &Path,
    registry: &Registry,
    ctx: &ToolCtx,
    s: &Checkpoint,
    call: ToolCall,
    protocol: ToolProtocol,
) -> Result<Execution> {
    let reject = |kind, message: String| {
        let mut r = rejected(&call, ctx, s, kind, message);
        r.protocol = protocol;
        r
    };
    if call.call_id.is_empty()
        || call.call_id.len() > 128
        || !call
            .call_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
        || call.name.len() > 240
        || !call.arguments.is_object()
        || call.arguments.to_string().len() > 32_000
    {
        return Ok(reject(
            ToolErrorKind::InvalidArguments,
            "工具调用编号或参数格式无效".into(),
        ));
    }
    let Some(tool) = registry.get(&call.name) else {
        return Ok(reject(
            ToolErrorKind::Unavailable,
            "工具没有可用执行器".into(),
        ));
    };
    if !tool.available_in_chat(ctx)
        || !effect_allowed(ctx, tool.as_ref())
        || (!s.allowed.is_empty() && !s.allowed.contains(&call.name))
        || !crate::capabilities::allowed(path, &call.name, "tool", ctx).await?
    {
        return Ok(reject(
            ToolErrorKind::PermissionDenied,
            "工具已停用或不在当前任务/会话范围内".into(),
        ));
    }
    let definition = register(path, tool.as_ref()).await?;
    if let Err(error) = crate::capabilities::validate(&definition.input_schema, &call.arguments) {
        return Ok(reject(ToolErrorKind::InvalidArguments, error.to_string()));
    }
    let id = ctx
        .task_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("任务工具调用缺少宿主任务编号"))?;
    let receipt =
        runtime::begin_tool_invocation(path, id, s, ctx, &call, &definition, protocol).await?;
    let started = Instant::now();
    let remaining = s.deadline.saturating_sub(crate::window::now()).max(1) as u64 * 1000;
    let timeout = Duration::from_millis(definition.policy.timeout_ms.min(remaining));
    let future =
        std::panic::AssertUnwindSafe(tool.call(ctx, call.arguments.clone())).catch_unwind();
    let run = async {
        match tokio::time::timeout(timeout, future).await {
            Ok(Ok(out)) => out,
            Ok(Err(_)) => {
                Err(ToolError::new(ToolErrorKind::Panic, "工具执行发生panic，结果需核实").into())
            }
            Err(_) => {
                Err(ToolError::new(ToolErrorKind::Timeout, "工具执行超时，结果需核实").into())
            }
        }
    };
    let outcome = if definition.policy.cancellation == ToolCancellation::DropFuture {
        tokio::select! {out=run=>out,signal=cancelled(path,id,s.revision)=>{
            signal?;Err(ToolError::new(ToolErrorKind::Cancelled,"任务已修改、暂停或取消，停止等待工具").into())
        }}
    } else {
        run.await
    };
    let mut report = ToolResult {
        call_id: call.call_id.clone(),
        name: call.name.clone(),
        task_id: id.into(),
        revision: s.revision,
        receipt_id: Some(receipt),
        status: ToolStatus::Succeeded,
        output: None,
        error: None,
        retryable: false,
        external_action_started: true,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    let mut audit = None;
    match outcome {
        Ok(output) => {
            if let Some(schema) = &definition.output_schema {
                if let Err(error) = crate::capabilities::validate(schema, &output.data) {
                    report.error = Some(ToolError::new(
                        ToolErrorKind::InvalidOutput,
                        format!("工具输出未通过Schema校验：{error}"),
                    ));
                }
            }
            let bytes = serde_json::to_vec(&output)?.len();
            if bytes > 2_000_000
                || output.artifacts.len() > 30
                || output.artifacts.iter().any(|p| p.len() > 4096)
            {
                report.error = Some(ToolError::new(
                    ToolErrorKind::InvalidOutput,
                    "工具结果超过宿主存储上限",
                ));
            } else {
                audit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tool.audit_details(&call.arguments, &output)
                }))
                .ok()
                .flatten()
                .filter(|v| v.to_string().len() <= 32_000);
                report.output = Some(output);
            }
        }
        Err(error) => {
            report.error = Some(
                error
                    .downcast_ref::<ToolError>()
                    .cloned()
                    .unwrap_or_else(|| {
                        ToolError::new(ToolErrorKind::ExecutionFailed, error.to_string())
                    }),
            )
        }
    }
    if let Some(error) = &mut report.error {
        error.message = error.message.chars().take(2000).collect();
    }
    if let Some(error) = &report.error {
        report.status = if matches!(
            error.kind,
            ToolErrorKind::InvalidArguments
                | ToolErrorKind::PermissionDenied
                | ToolErrorKind::Unavailable
        ) {
            report.external_action_started = false;
            ToolStatus::Rejected
        } else if definition.policy.effect == ToolEffect::ReadOnly {
            if error.kind == ToolErrorKind::Cancelled {
                ToolStatus::Cancelled
            } else {
                ToolStatus::Failed
            }
        } else {
            ToolStatus::Unknown
        };
        report.retryable = error.kind == ToolErrorKind::InvalidArguments;
    }
    Ok(Execution {
        call,
        protocol,
        result: report,
        audit_details: audit,
    })
}

async fn cancelled(path: &Path, id: &str, revision: i64) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        let mut c = crate::db::connect(path).await?;
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_runtime WHERE task_id=?1 AND revision=?2 AND phase='executing')").bind(id).bind(revision).fetch_one(&mut *c).await?;
        if !active {
            return Ok(());
        }
    }
}

pub async fn commit(path: &Path, id: &str, s: &Checkpoint, execution: &Execution) -> Result<()> {
    runtime::complete_tool_invocation(
        path,
        id,
        s,
        &execution.result,
        execution.audit_details.clone(),
    )
    .await
}
