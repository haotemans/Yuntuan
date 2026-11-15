//! Small, current capability hints for routing and Chat; not executable authority.
use super::{executor, Registry, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

pub async fn overview(path: &Path, registry: Option<&Registry>, ctx: &ToolCtx) -> Result<Value> {
    let Some(registry) = registry else {
        return Ok(json!({"known":false,"tools":[]}));
    };
    let definitions = executor::catalog(path, registry, ctx, &[]).await?;
    let total = definitions.len();
    let mut tools = vec![];
    let mut bytes = 0;
    for definition in definitions.into_iter().take(48) {
        let required = definition.input_schema["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let arguments = definition.input_schema["properties"].as_object().map(|properties| {
            properties.iter().take(12).map(|(name,schema)|json!({"name":name,"type":schema["type"],"required":required.iter().any(|r|r.as_str()==Some(name.as_str())),"description":schema["description"].as_str().unwrap_or("").chars().take(80).collect::<String>()})).collect::<Vec<_>>()
        }).unwrap_or_default();
        let item = json!({"name":definition.name,"description":definition.description.chars().take(200).collect::<String>(),"required":required.into_iter().take(12).collect::<Vec<_>>(),"arguments":arguments,"effect":definition.policy.effect});
        let size = item.to_string().chars().count();
        if bytes + size > 3600 {
            break;
        }
        bytes += size;
        tools.push(item);
    }
    Ok(
        json!({"known":true,"total":total,"omitted":total-tools.len(),"tools":tools,"dispatch":"现成Skill可直派；需要底层Tool或多步工作时由Router派Agent。目录不是授权，执行前会再次核对实际发起人、会话、参数及权限。省略不代表不可用。"}),
    )
}

/// Discovery failure must not become a false claim that no tools exist.
pub async fn refresh(
    path: &Path,
    registry: Option<&Registry>,
    snapshot: &mut crate::context_builder::ReplySnapshot,
    actor: &str,
) {
    let ctx = ToolCtx {
        task_id: None,
        chat_id: snapshot.chat_id.clone(),
        chat_type: snapshot.chat_type.clone(),
        sender_pid: actor.into(),
        locale: None,
    };
    snapshot.tool_capabilities = match overview(path, registry, &ctx).await {
        Ok(v) => v,
        Err(error) => {
            tracing::warn!(%error,"工具目录读取失败");
            json!({"known":false,"tools":[],"error":"目录暂时不可用，不能据此宣称没有工具"})
        }
    };
}

/// Capability hints are optional context, never a reason to discard a user's request.
pub fn shrink(value: &mut Value) -> bool {
    if !value["tools"]
        .as_array_mut()
        .is_some_and(|tools| tools.pop().is_some())
    {
        return false;
    }
    let visible = value["tools"].as_array().map(Vec::len).unwrap_or(0) as u64;
    value["omitted"] = json!(value["total"]
        .as_u64()
        .unwrap_or(visible)
        .saturating_sub(visible));
    true
}
