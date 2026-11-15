//! Both model wire protocols normalize to the same bounded Agent step.
use super::{parse_round, RoundOutput};
use crate::tools::{ToolDefinition, ToolProtocol};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const CONTROL: &str = "yuantuan_control";
pub fn alias(name: &str) -> String {
    use md5::{Digest, Md5};
    format!("tool_{:x}", Md5::digest(name.as_bytes()))
}
pub fn functions(catalog: &[ToolDefinition]) -> Result<Vec<Value>, ProtocolError> {
    let mut names = HashSet::new();
    if catalog.iter().any(|d| !names.insert(alias(&d.name))) {
        return Err(invalid("工具原生别名冲突，未发起模型调用"));
    }
    let mut out=catalog.iter().map(|d|json!({"type":"function","function":{"name":alias(&d.name),"description":format!("{} [{}] policy={}",d.description,d.name,serde_json::to_string(&d.policy).unwrap()),"parameters":d.input_schema,"strict":false}})).collect::<Vec<_>>();
    out.push(json!({"type":"function","function":{"name":CONTROL,"description":"报告进度(reply)、完成(done)、等待用户补充(wait_input)或拆分子任务(delegate)。只交给宿主，不直接发用户消息。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["reply","done","wait_input","delegate"]},"text":{"type":"string"},"children":{"type":"array","items":{"type":"string"},"maxItems":2},"reason":{"type":"string"}},"required":["action"],"additionalProperties":false},"strict":false}}));
    Ok(out)
}
#[derive(Debug)]
pub struct ProtocolError {
    pub message: String,
    pub repairable: bool,
}
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ProtocolError {}
fn invalid(message: &str) -> ProtocolError {
    ProtocolError {
        message: message.into(),
        repairable: true,
    }
}

pub fn native(response: &Value, catalog: &[ToolDefinition]) -> Result<RoundOutput, ProtocolError> {
    let choice = response
        .pointer("/choices/0")
        .ok_or_else(|| invalid("原生响应缺少choices[0]"))?;
    let message = &choice["message"];
    if choice["finish_reason"] == "length"
        || choice["finish_reason"] == "content_filter"
        || message["refusal"].as_str().is_some_and(|s| !s.is_empty())
    {
        return Err(ProtocolError {
            message: "模型拒绝或输出截断，未执行工具".into(),
            repairable: false,
        });
    }
    if !matches!(
        choice["finish_reason"].as_str(),
        Some("stop" | "tool_calls")
    ) {
        return Err(invalid("原生响应结束标志无效"));
    }
    if !message["tool_calls"].is_null() && !message["tool_calls"].is_array() {
        return Err(invalid("tool_calls必须是数组"));
    }
    if !message["function_call"].is_null() {
        return Err(invalid("不支持缺少调用编号的旧function_call响应"));
    }
    if let Some(calls) = message["tool_calls"].as_array().filter(|a| !a.is_empty()) {
        if calls.len() != 1 {
            return Err(invalid("每轮只允许一个工具调用，未执行批量调用"));
        }
        let call = &calls[0];
        let id = call["id"]
            .as_str()
            .ok_or_else(|| invalid("原生调用缺少id"))?;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
        {
            return Err(invalid("原生调用编号无效"));
        }
        if call["type"] != "function" {
            return Err(invalid("只支持function工具调用"));
        }
        let name = call["function"]["name"]
            .as_str()
            .ok_or_else(|| invalid("原生调用缺少工具名"))?;
        let raw = call["function"]["arguments"]
            .as_str()
            .ok_or_else(|| invalid("工具arguments必须是JSON字符串"))?;
        if raw.len() > 32_000 {
            return Err(invalid("工具参数超过32KB"));
        }
        let args: Value = serde_json::from_str(raw).map_err(|_| invalid("工具参数不是完整JSON"))?;
        if !args.is_object() {
            return Err(invalid("工具参数必须是对象"));
        }
        if name == CONTROL {
            let mut step = parse_round(raw).map_err(|_| invalid("控制调用参数不符合Agent协议"))?;
            if !["reply", "done", "wait_input", "delegate"].contains(&step.action.as_str())
                || step.tool_name.is_some()
                || step.tool_args.is_some()
            {
                return Err(invalid("控制调用不能伪装为外部工具"));
            }
            step.call_id = Some(id.into());
            return Ok(step);
        }
        let definition = catalog
            .iter()
            .find(|d| alias(&d.name) == name)
            .ok_or_else(|| invalid("模型调用了未声明工具，未执行"))?;
        return Ok(RoundOutput {
            action: "tool_call".into(),
            tool_name: Some(definition.name.clone()),
            tool_args: Some(args),
            text: None,
            reason: None,
            children: vec![],
            call_id: Some(id.into()),
        });
    }
    if choice["finish_reason"] == "tool_calls" {
        return Err(invalid("tool_calls结束标志没有完整调用"));
    }
    let text = message["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("原生响应没有正文或工具调用"))?;
    if serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|v| v["action"].is_string())
    {
        return Err(invalid(
            "原生模式请使用声明的函数或最终正文，不输出旧动作JSON",
        ));
    }
    Ok(RoundOutput {
        action: "done".into(),
        text: Some(text.into()),
        tool_name: None,
        tool_args: None,
        reason: None,
        children: vec![],
        call_id: None,
    })
}

/// Reconstruct only complete, persisted call/result pairs. Compaction cannot leave dangling IDs.
pub fn transcript(history: &[Value]) -> Result<Vec<Value>, ProtocolError> {
    let mut out = vec![];
    let mut ids = HashSet::new();
    for record in history {
        if record["role"] != "tool_result" || record["protocol"] != json!(ToolProtocol::Native) {
            // Preserve chronology, including repair feedback after a result. The brief stays
            // in a separate stable first user message; results remain typed data.
            let role = if record["role"] == "assistant" {
                "assistant"
            } else {
                "user"
            };
            let content = record["content"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| record.to_string());
            out.push(json!({"role":role,"content":content}));
            continue;
        }
        let call = &record["call"];
        let result = &record["result"];
        let id = call["call_id"]
            .as_str()
            .ok_or_else(|| invalid("检查点缺少工具调用编号"))?;
        if !ids.insert(id.to_string()) || result["call_id"] != id || result["name"] != call["name"]
        {
            return Err(invalid("检查点调用/结果不匹配"));
        }
        let name = call["name"]
            .as_str()
            .ok_or_else(|| invalid("检查点缺少工具名称"))?;
        out.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":id,"type":"function","function":{"name":alias(name),"arguments":call["arguments"].to_string()}}]}));
        out.push(json!({"role":"tool","tool_call_id":id,"content":result.to_string()}));
    }
    Ok(out)
}
