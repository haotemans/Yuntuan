use super::{ingest, OfficialQq};
use anyhow::{anyhow, bail, ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    net::TcpStream,
    time::{self, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const HANDSHAKE: Duration = Duration::from_secs(20);
const INTENTS: u64 = 1 << 25;

#[derive(Default)]
pub(super) struct Session {
    id: Option<String>,
    sequence: Option<u64>,
}

impl Session {
    fn identify(&self, token: &str) -> Value {
        let token = format!("QQBot {token}");
        if let (Some(id), Some(seq)) = (&self.id, self.sequence) {
            json!({"op":6,"d":{"token":token,"session_id":id,"seq":seq}})
        } else {
            json!({"op":2,"d":{"token":token,"intents":INTENTS,"shard":[0,1]}})
        }
    }
}

#[derive(Debug)]
struct Closed(u16);

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QQ 官方网关断开（{}）", self.0)
    }
}

impl std::error::Error for Closed {}

impl OfficialQq {
    pub(super) async fn run_gateway(&self) {
        let mut session = Session::default();
        let mut delay = Duration::from_secs(2);
        loop {
            let Err(error) = self.connect_gateway(&mut session).await else {
                return;
            };
            if self.status().state == "online" {
                delay = Duration::from_secs(2);
            }
            let code = error.downcast_ref::<Closed>().map(|e| e.0);
            if matches!(code, Some(4914 | 4915)) {
                self.set_status(
                    "error",
                    Some("请核对 QQ 开放平台账号状态和群聊/单聊权限".into()),
                );
                return;
            }
            if matches!(code, Some(4004 | 4006 | 4007 | 4009)) {
                session = Session::default();
            }
            if code == Some(4004) {
                self.0.token.lock().await.take();
            }
            let wait = if code == Some(4008) {
                Duration::from_secs(60)
            } else {
                delay
            };
            tracing::warn!(error=%error, "QQ 官方机器人将重连");
            self.set_status("reconnecting", Some(error.to_string()));
            time::sleep(wait).await;
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    }

    async fn connect_gateway(&self, session: &mut Session) -> Result<()> {
        let token = self.token().await?;
        let url = self.gateway_url(&token).await?;
        let options = WebSocketConfig::default()
            .max_message_size(Some(1024 * 1024))
            .max_frame_size(Some(1024 * 1024));
        let (mut socket, _) = time::timeout(
            HANDSHAKE,
            connect_async_with_config(url.as_str(), Some(options), false),
        )
        .await
        .context("QQ 官方网关握手超时")?
        .context("QQ 官方网关握手失败")?;
        self.run_socket(&mut socket, session, &token).await
    }

    pub(super) async fn run_socket(
        &self,
        socket: &mut Socket,
        session: &mut Session,
        token: &str,
    ) -> Result<()> {
        let hello = time::timeout(HANDSHAKE, next_payload(socket))
            .await
            .context("QQ 官方网关未发送握手信息")??;
        ensure!(hello["op"] == 10, "QQ 官方网关握手事件无效");
        let millis = hello["d"]["heartbeat_interval"]
            .as_u64()
            .filter(|v| (20..=120_000).contains(v))
            .context("QQ 官方网关的心跳周期无效")?;
        send_payload(socket, session.identify(token)).await?;
        let interval = Duration::from_millis(millis);
        let mut heartbeat = time::interval_at(Instant::now() + interval, interval);
        heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let deadline = time::sleep(HANDSHAKE);
        tokio::pin!(deadline);
        let mut ready = false;
        let mut acknowledged = true;
        loop {
            tokio::select! {
                _ = &mut deadline, if !ready => bail!("QQ 官方网关未确认连接"),
                _ = heartbeat.tick() => {
                    ensure!(acknowledged, "QQ 官方网关未确认心跳");
                    send_payload(socket, json!({"op":1,"d":session.sequence})).await?;
                    acknowledged = false;
                }
                result = next_payload(socket) => {
                    let event = result?;
                    match event["op"].as_u64() {
                        Some(0) => {
                            let kind = event["t"].as_str().unwrap_or_default();
                            match kind {
                                "READY" => {
                                    session.id = Some(event["d"]["session_id"].as_str()
                                        .filter(|id| !id.is_empty() && id.len() <= 512)
                                        .context("QQ 官方网关会话编号无效")?.to_owned());
                                    ready = true;
                                }
                                "RESUMED" => ready = session.id.is_some(),
                                _ if ready => ingest::accept(self, kind, &event["d"]).await?,
                                _ => continue,
                            }
                            if let Some(seq) = event["s"].as_u64() { session.sequence = Some(seq); }
                            if ready && self.status().state != "online" {
                                self.set_status("online", None);
                            }
                        }
                        Some(1) => send_payload(socket, json!({"op":1,"d":session.sequence})).await?,
                        Some(7) => bail!("QQ 官方网关请求重连"),
                        Some(9) => {
                            if event["d"] != true { *session = Session::default(); }
                            bail!("QQ 官方网关会话需要重新建立");
                        }
                        Some(11) => acknowledged = true,
                        _ => {}
                    }
                }
            }
        }
    }
}

async fn send_payload(socket: &mut Socket, value: Value) -> Result<()> {
    time::timeout(
        HANDSHAKE,
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .context("QQ 官方网关写入超时")?
    .context("QQ 官方网关写入失败")
}

async fn next_payload(socket: &mut Socket) -> Result<Value> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).context("QQ 官方网关事件格式无效");
            }
            Some(Ok(Message::Close(frame))) => {
                return Err(Closed(frame.map(|f| f.code.into()).unwrap_or(1000)).into());
            }
            Some(Ok(_)) => {}
            Some(Err(_)) => bail!("QQ 官方网关读取失败"),
            None => return Err(anyhow!("QQ 官方网关已断开")),
        }
    }
}
