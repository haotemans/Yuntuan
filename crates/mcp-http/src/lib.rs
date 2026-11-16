//! Bounded Streamable HTTP MCP transport; no implicit retries, shells or credential export.
use anyhow::{bail, ensure, Context, Result};
use reqwest::{header, Client, Url};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
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
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.query().is_none(),
            "远程 MCP 必须使用无内嵌凭据的 HTTPS 地址"
        );
        ensure!(
            !token.trim().is_empty() && token.len() <= 8192,
            "MCP Token 尚未配置或过长"
        );
        let mut authorization =
            header::HeaderValue::from_str(token.trim()).context("MCP Token 格式无效")?;
        authorization.set_sensitive(true);
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
    async fn exchange(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
        notification: bool,
    ) -> Result<(Value, Option<String>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut payload = json!({"jsonrpc":"2.0","method":method,"params":params});
        if !notification {
            payload["id"] = json!(id);
        }
        let mut request = self
            .http
            .post(self.url.clone())
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .header("MCP-Protocol-Version", VERSION)
            .json(&payload);
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        // Never surface reqwest's URL/body/source chain, which may include upstream private data.
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("MCP 连接失败"))?;
        let code = response.status();
        ensure!(
            code.is_success(),
            "远程 MCP 响应 HTTP {}（请检查授权；不会自动重试）",
            code.as_u16()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= RESPONSE_LIMIT as u64),
            "MCP 响应过大"
        );
        let sid = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() <= 1024)
            .map(str::to_owned);
        if notification && matches!(code.as_u16(), 202 | 204) {
            return Ok((Value::Null, sid));
        }
        let sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("text/event-stream"));
        let mut bytes = Vec::new();
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|_| anyhow::anyhow!("MCP 响应读取失败"))?;
            let Some(chunk) = chunk else { break };
            ensure!(
                bytes.len() + chunk.len() <= RESPONSE_LIMIT,
                "MCP 响应超过 1 MiB"
            );
            bytes.extend_from_slice(&chunk);
            if sse {
                if let Some(reply) = parse_sse(&bytes, id)? {
                    return decode(reply, notification, id).map(|v| (v, sid));
                }
            }
        }
        if notification && bytes.is_empty() {
            return Ok((Value::Null, sid));
        }
        ensure!(!sse, "MCP SSE 未返回匹配的完整 JSON-RPC 结果");
        let reply: Value = serde_json::from_slice(&bytes).context("MCP 返回无效 JSON")?;
        decode(reply, notification, id).map(|v| (v, sid))
    }
    async fn ensure_ready(&self) -> Result<Option<String>> {
        let mut guard = self.session.lock().await;
        if self.ready.load(Ordering::Acquire) {
            return Ok(guard.clone());
        }
        let (init, sid) = self
            .exchange(
                "initialize",
                json!({"protocolVersion":VERSION,"capabilities":{},
            "clientInfo":{"name":"Yuntuan","version":"0.1.0"}}),
                None,
                false,
            )
            .await?;
        ensure!(
            init["protocolVersion"] == VERSION,
            "MCP 服务返回了当前客户端不支持的协议版本"
        );
        self.exchange("notifications/initialized", json!({}), sid.as_deref(), true)
            .await?;
        *guard = sid.clone();
        self.ready.store(true, Ordering::Release);
        Ok(sid)
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let sid = self.ensure_ready().await?;
        self.exchange(method, params, sid.as_deref(), false)
            .await
            .map(|(v, _)| v)
    }
    pub async fn list_tools(&self) -> Result<Vec<Value>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..8 {
            let params = cursor.as_ref().map_or(json!({}), |c| json!({"cursor":c}));
            let result = self.call("tools/list", params).await?;
            let page = result["tools"].as_array().context("MCP 工具目录格式无效")?;
            ensure!(tools.len() + page.len() <= 200, "MCP 工具目录超过 200 项");
            tools.extend(page.iter().cloned());
            let next = result["nextCursor"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            ensure!(next.is_none() || next != cursor, "MCP 工具目录重复分页游标");
            cursor = next;
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        bail!("MCP 工具目录分页超过限制")
    }
    /// Caller validates the discovered descriptor and permissions before invoking.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        ensure!(
            !name.is_empty() && name.len() <= 128 && arguments.is_object(),
            "MCP 工具名或参数无效"
        );
        let result = self
            .call("tools/call", json!({"name":name,"arguments":arguments}))
            .await?;
        ensure!(result["isError"] != true, "MCP 工具返回错误");
        Ok(result)
    }
}
fn decode(reply: Value, notification: bool, id: u64) -> Result<Value> {
    if notification {
        return Ok(Value::Null);
    }
    ensure!(
        reply["jsonrpc"] == "2.0" && reply["id"].as_u64() == Some(id),
        "MCP JSON-RPC 响应编号不匹配"
    );
    if reply.get("error").is_some() {
        bail!(
            "MCP 调用失败（错误码 {}）",
            reply["error"]["code"].as_i64().unwrap_or(-1)
        );
    }
    reply.get("result").cloned().context("MCP 响应缺少 result")
}
fn parse_sse(bytes: &[u8], id: u64) -> Result<Option<Value>> {
    // Incomplete UTF-8 and events must wait for another network chunk.
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) if e.error_len().is_none() => return Ok(None),
        Err(_) => bail!("MCP SSE 不是 UTF-8"),
    };
    let text = text.replace("\r\n", "\n");
    for event in text.split_inclusive("\n\n").filter(|e| e.ends_with("\n\n")) {
        let data = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&data).context("MCP SSE 事件包含无效 JSON")?;
        if value["id"].as_u64() == Some(id)
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
#[cfg(test)]
mod tests;
