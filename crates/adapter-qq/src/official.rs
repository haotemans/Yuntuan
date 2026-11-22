//! QQ Open Platform gateway and passive replies. OpenIDs stay in this adapter.
mod gateway;
mod ingest;

use anyhow::{anyhow, ensure, Context, Result};
use reqwest::{Client, Response, Url};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use yuantuan_core::{
    db,
    event::EventBus,
    platform::{OfficialQqConfig, PlatformStatus, QQ_OFFICIAL_ID},
    reply_engine::{ChatType, DeliveryUnconfirmed, SendOperation, SendRequest},
};

const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const API_ROOT: &str = "https://api.sgroup.qq.com";

#[derive(Clone)]
pub struct OfficialQq(Arc<Inner>);

struct Inner {
    cfg: OfficialQqConfig,
    http: Client,
    db_path: PathBuf,
    bus: EventBus,
    token: Mutex<Option<Token>>,
    status: RwLock<PlatformStatus>,
    token_url: String,
    api_root: String,
}

struct Token {
    value: String,
    refresh_at: Instant,
}

impl OfficialQq {
    pub fn new(cfg: OfficialQqConfig, db_path: PathBuf, bus: EventBus) -> Result<Self> {
        cfg.validate()?;
        let http = Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let status = PlatformStatus {
            id: QQ_OFFICIAL_ID,
            state: if cfg.enabled {
                "connecting"
            } else {
                "disabled"
            },
            account: (!cfg.app_id.is_empty()).then(|| cfg.app_id.clone()),
            detail: None,
        };
        Ok(Self(Arc::new(Inner {
            cfg,
            http,
            db_path,
            bus,
            token: Mutex::new(None),
            status: RwLock::new(status),
            token_url: TOKEN_URL.into(),
            api_root: API_ROOT.into(),
        })))
    }

    pub fn status(&self) -> PlatformStatus {
        self.0.status.read().unwrap().clone()
    }

    fn set_status(&self, state: &'static str, detail: Option<String>) {
        let mut status = self.0.status.write().unwrap();
        status.state = state;
        status.detail = detail;
    }

    pub fn spawn(&self) -> tokio::task::JoinHandle<()> {
        let adapter = self.clone();
        tokio::spawn(async move {
            if adapter.0.cfg.enabled {
                adapter.run_gateway().await;
            }
        })
    }

    async fn token(&self) -> Result<String> {
        let mut cached = self.0.token.lock().await;
        if let Some(token) = cached.as_ref().filter(|t| t.refresh_at > Instant::now()) {
            return Ok(token.value.clone());
        }
        let response = self
            .0
            .http
            .post(&self.0.token_url)
            .json(&json!({"appId":self.0.cfg.app_id,"clientSecret":self.0.cfg.app_secret}))
            .send()
            .await
            .map_err(|_| anyhow!("QQ 官方机器人令牌服务连接失败"))?;
        let value = response_json(response, "获取 QQ 官方机器人访问令牌").await?;
        let token = value["access_token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096 && s.bytes().all(|b| b.is_ascii_graphic()))
            .context("QQ 未返回有效访问令牌，请核对 AppID 和 AppSecret")?;
        let expires = value["expires_in"]
            .as_u64()
            .or_else(|| value["expires_in"].as_str()?.parse().ok())
            .filter(|s| *s > 0)
            .context("QQ 访问令牌有效期无效")?
            .min(86_400);
        *cached = Some(Token {
            value: token.to_owned(),
            refresh_at: Instant::now() + Duration::from_secs(expires - (expires / 3).min(300)),
        });
        Ok(token.to_owned())
    }

    fn request(&self, method: reqwest::Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.0
            .http
            .request(method, format!("{}{path}", self.0.api_root))
            .header(reqwest::header::AUTHORIZATION, format!("QQBot {token}"))
    }

    async fn gateway_url(&self, token: &str) -> Result<String> {
        let response = self
            .request(reqwest::Method::GET, "/gateway", token)
            .send()
            .await
            .map_err(|_| anyhow!("无法连接 QQ 官方网关入口"))?;
        if response.status().as_u16() == 401 {
            self.0.token.lock().await.take();
        }
        let value = response_json(response, "获取 QQ 官方网关地址").await?;
        let url = Url::parse(value["url"].as_str().context("QQ 未返回网关地址")?)
            .map_err(|_| anyhow!("QQ 官方网关地址无效"))?;
        ensure!(
            url.scheme() == "wss"
                && url.host_str().is_some_and(|host| {
                    host == "api.sgroup.qq.com" || host.ends_with(".sgroup.qq.com")
                })
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443),
            "QQ 返回了非官方安全网关地址"
        );
        Ok(url.into())
    }

    pub async fn send(&self, req: SendRequest) -> Result<Value> {
        ensure!(self.0.cfg.enabled, "QQ 官方机器人未启用");
        ensure!(
            req.operation == SendOperation::Message,
            "QQ 官方机器人当前支持文本回复"
        );
        let source = req.platform_reply.context("缺少 QQ 官方机器人回复来源")?;
        ensure!(
            source.platform == QQ_OFFICIAL_ID,
            "QQ 官方机器人回复通道不匹配"
        );
        let mut content = String::new();
        for segment in req.segments.as_array().context("回复消息格式无效")? {
            ensure!(segment["type"] == "text", "QQ 官方机器人当前支持文本回复");
            content.push_str(segment["data"]["text"].as_str().unwrap_or_default());
        }
        ensure!(!content.trim().is_empty(), "QQ 官方机器人回复正文为空");
        let (scope, kind, ttl) = match req.chat_type {
            ChatType::Group => ("groups", "group", 300),
            ChatType::Private => ("users", "private", 3600),
        };
        let token = self.token().await?;
        let mut c = db::connect(&self.0.db_path).await?;
        let (recipient, msg_id, seq): (String, String, i64) = sqlx::query_as(
            "UPDATE platform_messages SET next_seq=next_seq+1
             WHERE platform=?1 AND account=?2 AND msg_id=?3 AND next_seq<=5
             AND EXISTS(SELECT 1 FROM messages m WHERE m.msg_id=?3 AND m.chat_id=?4
                 AND m.chat_type=?5 AND m.ts>=unixepoch()-?6 AND m.sender_pid!='self')
             RETURNING recipient,message_key,next_seq-1",
        )
        .bind(QQ_OFFICIAL_ID)
        .bind(&self.0.cfg.app_id)
        .bind(source.anchor_msg_id)
        .bind(&source.chat_id)
        .bind(kind)
        .bind(ttl)
        .fetch_optional(&mut *c)
        .await?
        .context("QQ 官方回复来源不匹配、已过期或达到回复次数上限")?;
        ensure!(ingest::valid_openid(&recipient), "QQ 官方回复目标无效");
        drop(c);
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v2/{scope}/{recipient}/messages"),
                &token,
            )
            .json(&json!({"content":content,"msg_type":0,"msg_id":msg_id,"msg_seq":seq}))
            .send()
            .await
            .map_err(|_| DeliveryUnconfirmed("QQ 官方回复未收到确认".into()))?;
        if response.status().as_u16() == 401 {
            self.0.token.lock().await.take();
        }
        let result = response_json(response, "发送 QQ 官方回复").await?;
        let id = result["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
            .ok_or_else(|| DeliveryUnconfirmed("QQ 官方回复缺少消息回执".into()))?;
        Ok(json!({"platform_message_id":id}))
    }
}

async fn response_json(mut response: Response, action: &str) -> Result<Value> {
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("{action}：未能读取响应"))?
    {
        ensure!(body.len() + chunk.len() <= 64 * 1024, "{action}：响应过大");
        body.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if let Some(code) = value["code"].as_i64().filter(|code| *code != 0) {
        return Err(anyhow!("{action}失败（QQ 错误码 {code}，HTTP {status}）"));
    }
    ensure!(
        status.is_success() && value.is_object(),
        "{action}失败（HTTP {status}）"
    );
    Ok(value)
}

#[cfg(test)]
mod tests;
