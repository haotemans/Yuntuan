//! Shared, bounded Streamable HTTP MCP transport for official service connectors.
//! The transport never invents credentials, runs shell commands or retries writes.
use anyhow::{bail, Context, Result};
use reqwest::{header, Client, Url};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;

const VERSION: &str = "2025-06-18";
const RESPONSE_LIMIT: usize = 1024 * 1024;

pub struct HttpMcpClient {
    url: Url,
    authorization: header::HeaderValue,
    http: Client,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
    ready: AtomicBool,
}

impl HttpMcpClient {
    pub fn new(endpoint: &str, token: &str) -> Result<Self> {
        let url = Url::parse(endpoint).context("MCP 地址无效")?;
        if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty()
            || url.password().is_some() || url.fragment().is_some()
        {
            bail!("远程 MCP 必须使用无内嵌凭据的 HTTPS 地址");
        }
        let authorization = header::HeaderValue::from_str(token.trim())
            .context("MCP Token 格式无效")?;
        if token.trim().is_empty() {
            bail!("MCP Token 尚未配置");
        }
        Ok(Self {
            url,
            authorization,
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(35))
                .build()?,
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
            ready: AtomicBool::new(false),
        })
    }

    async fn exchange(&self, method: &str, params: Value, session: Option<&str>, notification: bool) -> Result<(Value, Option<String>)> {
        let mut payload = json!({"jsonrpc":"2.0","method":method,"params":params});
        if !notification {
            payload["id"] = json!(self.next_id.fetch_add(1, Ordering::Relaxed));
        }
        let mut request = self.http.post(self.url.clone())
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .header("MCP-Protocol-Version", VERSION)
            .json(&payload);
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request.send().await.context("MCP 连接失败")?;
        let code = response.status();
        if !code.is_success() {
            bail!("远程 MCP 响应 HTTP {}", code.as_u16());
        }
        if response.content_length().is_some_and(|len| len > RESPONSE_LIMIT as u64) {
            bail!("MCP 响应过大");
        }
        let sid = response.headers().get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok()).map(str::to_owned);
        let content_type = response.headers().get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        let reply = if content_type.contains("text/event-stream") {
            // Streamable HTTP SSE may stay open after the JSON-RPC response.
            // Stop reading as soon as one complete result event is received.
            let mut body = Vec::new();
            let mut response = response;
            loop {
                let Some(chunk) = response.chunk().await.context("MCP SSE 读取失败")? else {
                    bail!("MCP SSE 未返回 JSON-RPC 结果");
                };
                if body.len() + chunk.len() > RESPONSE_LIMIT {
                    bail!("MCP SSE 超过 1 MiB");
                }
                body.extend_from_slice(&chunk);
                if let Ok(result) = parse_sse(&body) {
                    break result;
                }
            }
        } else {
            let bytes = response.bytes().await.context("MCP 响应读取失败")?;
            if bytes.len() > RESPONSE_LIMIT {
                bail!("MCP 响应超过 1 MiB");
            }
            if bytes.is_empty() && notification {
                return Ok((Value::Null, sid));
            }
            serde_json::from_slice::<Value>(&bytes).context("MCP 返回无效 JSON")?
        };
        if !notification {
            if let Some(err) = reply.get("error") {
                bail!("MCP 调用失败：{}", err.get("message").and_then(Value::as_str).unwrap_or("未知错误"));
            }
            return Ok((reply.get("result").cloned().context("MCP 响应缺少 result")?, sid));
        }
        Ok((Value::Null, sid))
    }

    async fn ensure_ready(&self) -> Result<Option<String>> {
        let mut guard = self.session.lock().await;
        if self.ready.load(Ordering::Acquire) {
            return Ok(guard.clone());
        }
        let (init, sid) = self.exchange("initialize", json!({
            "protocolVersion":VERSION,
            "capabilities":{},
            "clientInfo":{"name":"Yuntuan","version":"0.1.0"}
        }), None, false).await?;
        if init.get("protocolVersion").and_then(Value::as_str).is_none() {
            bail!("MCP 初始化响应缺少协议版本");
        }
        self.exchange("notifications/initialized", json!({}), sid.as_deref(), true).await?;
        *guard = sid.clone();
        self.ready.store(true, Ordering::Release);
        Ok(sid)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let sid = self.ensure_ready().await?;
        let (result, _) = self.exchange(method, params, sid.as_deref(), false).await?;
        Ok(result)
    }

    pub async fn list_tools(&self) -> Result<Vec<Value>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..8 {
            let params = cursor.as_ref().map_or(json!({}), |c| json!({"cursor": c}));
            let result = self.call("tools/list", params).await?;
            let page = result["tools"].as_array().context("MCP 工具目录格式无效")?;
            if tools.len() + page.len() > 200 {
                bail!("MCP 工具目录超过 200 项");
            }
            tools.extend(page.iter().cloned());
            cursor = result.get("nextCursor").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        bail!("MCP 工具目录分页超过限制");
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        if name.is_empty() || name.len() > 128 || !arguments.is_object() {
            bail!("MCP 工具名或参数无效");
        }
        let allowed = self.list_tools().await?.iter()
            .any(|item| item["name"].as_str() == Some(name));
        if !allowed {
            bail!("MCP 未提供指定工具");
        }
        let result = self.call("tools/call", json!({"name":name,"arguments":arguments})).await?;
        if result["isError"].as_bool() == Some(true) {
            bail!("MCP 工具返回错误");
        }
        Ok(result)
    }
}

fn parse_sse(bytes: &[u8]) -> Result<Value> {
    let stream = std::str::from_utf8(bytes).context("MCP SSE 不是 UTF-8")?;
    let mut data = String::new();
    for line in stream.lines().chain(std::iter::once("")) {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.trim_start());
            data.push('\n');
        } else if line.is_empty() && !data.is_empty() {
            let raw = data.trim();
            if let Ok(value) = serde_json::from_str::<Value>(raw) {
                if value.get("result").is_some() || value.get("error").is_some() {
                    return Ok(value);
                }
            }
            data.clear();
        }
    }
    bail!("MCP SSE 未返回 JSON-RPC 结果");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_parses_multiline_and_rejects_empty() {
        let data = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"result\":{\"tools\":[]}}\n\n";
        assert_eq!(parse_sse(data).unwrap()["result"]["tools"], json!([]));
        assert!(parse_sse(b": keepalive\n\n").is_err());
    }
    #[test]
    fn rejects_http_and_embedded_credentials() {
        assert!(HttpMcpClient::new("http://docs.qq.com/openapi/mcp","token").is_err());
        assert!(HttpMcpClient::new("https://x:y@docs.qq.com/openapi/mcp","token").is_err());
        assert!(HttpMcpClient::new("https://docs.qq.com/openapi/mcp","").is_err());
    }
}
