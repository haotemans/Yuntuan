//! yuantuan-adapter-qq：OneBot 11 反向 WS 服务器（与 AstrBot 的 aiocqhttp 同形态）。
//! yuantuan 起 axum 监听 `GET /ws`（默认 127.0.0.1:6199），NapCat 作为 Websockets客户端连入。
//! 铁律：core 不见 CQ 码（段数组）、echo 回执路由、掉线仅记录等待 NapCat 重连。

mod client;
mod ingest;
mod moderation;
pub mod official;
mod qzone;
mod server;
pub use moderation::moderation_service;
pub use official::official_gateway;
pub use qzone::qzone_service;

pub use client::{
    onebot_gateway, send_fn, shared_token, AdapterHandle, NapcatConfig, NapcatSender, SharedToken,
};
pub use server::spawn;

/// Platform-specific avatar URL formatter registered into the core/webui memory interface.
pub fn avatar_resolver() -> yuantuan_core::memory::AvatarResolver {
    std::sync::Arc::new(|platform, uid| {
        if platform == "qq" && !uid.is_empty() && uid.bytes().all(|c| c.is_ascii_digit()) {
            Some(format!("https://q1.qlogo.cn/g?b=qq&nk={uid}&s=100"))
        } else {
            None
        }
    })
}
