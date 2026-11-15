//! Deliberately bounded JSON Schema subset; reports paths without echoing argument values.
use anyhow::{ensure, Result};
use serde_json::Value;

pub fn validate(schema: &Value, args: &Value) -> Result<()> {
    check(schema, args, "", 0)
}
fn segment(name: &str) -> String {
    name.replace('~', "~0")
        .replace('/', "~1")
        .chars()
        .take(120)
        .collect()
}
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(v) if v.is_i64() || v.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
fn matches(value: &Value, expected: &str) -> bool {
    kind(value) == expected || expected == "number" && value.is_number()
}
fn check(schema: &Value, args: &Value, path: &str, depth: usize) -> Result<()> {
    let at = if path.is_empty() { "/" } else { path };
    ensure!(depth <= 32, "参数 {at}：嵌套超过32层");
    if let Some(t) = schema.get("type") {
        let valid = match t {
            Value::String(t) => matches(args, t),
            Value::Array(types) => types
                .iter()
                .any(|t| t.as_str().is_some_and(|t| matches(args, t))),
            _ => false,
        };
        ensure!(
            valid,
            "参数 {at}：工具参数类型不匹配，需要 {t}，收到 {}",
            kind(args)
        );
    }
    if let Some(values) = schema["enum"].as_array() {
        ensure!(
            values.contains(args),
            "参数 {at}：参数不在允许值中，请使用工具Schema的enum选项"
        );
    }
    if let Some(required) = schema["required"].as_array() {
        for key in required.iter().filter_map(Value::as_str) {
            ensure!(
                args.get(key).is_some(),
                "参数 {}/{}：缺少必填参数",
                path,
                segment(key)
            );
        }
    }
    if let Some(object) = args.as_object() {
        let properties = schema["properties"].as_object();
        for (key, value) in object {
            let next = format!("{}/{}", path, segment(key));
            if let Some(child) = properties.and_then(|p| p.get(key)) {
                check(child, value, &next, depth + 1)?;
            } else if schema["additionalProperties"] == false {
                anyhow::bail!("参数 {next}：不接受此字段，请使用工具Schema中的参数名");
            } else if schema["additionalProperties"].is_object() {
                check(&schema["additionalProperties"], value, &next, depth + 1)?;
            }
        }
    }
    if let Some(values) = args.as_array() {
        let n = values.len() as u64;
        ensure!(
            schema["minItems"].as_u64().is_none_or(|min| n >= min)
                && schema["maxItems"].as_u64().is_none_or(|max| n <= max),
            "参数 {at}：数组项数超出允许范围（minItems={}，maxItems={}）",
            schema["minItems"],
            schema["maxItems"]
        );
        if let Some(child) = schema.get("items") {
            for (i, value) in values.iter().enumerate() {
                check(child, value, &format!("{path}/{i}"), depth + 1)?;
            }
        }
    }
    if let Some(text) = args.as_str() {
        let n = text.chars().count() as u64;
        ensure!(
            schema["minLength"].as_u64().is_none_or(|min| n >= min)
                && schema["maxLength"].as_u64().is_none_or(|max| n <= max),
            "参数 {at}：文字长度超出允许范围（minLength={}，maxLength={}）",
            schema["minLength"],
            schema["maxLength"]
        );
    }
    if let Some(n) = args.as_f64() {
        ensure!(
            schema["minimum"].as_f64().is_none_or(|min| n >= min)
                && schema["maximum"].as_f64().is_none_or(|max| n <= max),
            "参数 {at}：参数超出范围（minimum={}，maximum={}）",
            schema["minimum"],
            schema["maximum"]
        );
    }
    Ok(())
}
