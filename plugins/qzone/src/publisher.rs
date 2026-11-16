//! Durable receipts: reserve before publishing; unknown outcomes never auto-replay.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use yuantuan_plugin_api::*;

const LEDGER: &str = "publications";
const MAX_RECORDS: usize = 200;

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    key: String,
    fingerprint: String,
    day: u64,
    created_at: u64,
    qq: u64,
    preview: String,
    image_count: usize,
    visibility: QzoneVisibility,
    status: String,
    receipt: Option<Value>,
}
#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    records: Vec<Record>,
}

pub(super) struct Publisher {
    pub host: Arc<dyn HostServices>,
    pub visibility: QzoneVisibility,
    limit: usize,
    serial: Mutex<()>,
}
fn err(kind: ToolErrorKind, message: &str) -> anyhow::Error {
    ToolError::new(kind, message).into()
}
fn unknown() -> anyhow::Error {
    err(
        ToolErrorKind::ResultUnknown,
        "该说说发布结果待核实，请先查看QQ空间；不会自动重发",
    )
}

impl Publisher {
    pub fn new(host: Arc<dyn HostServices>, visibility: QzoneVisibility, limit: usize) -> Self {
        Self {
            host,
            visibility,
            limit,
            serial: Mutex::new(()),
        }
    }
    async fn load(&self) -> Result<Ledger> {
        let ledger: Ledger = match self.host.read_data(LEDGER).await? {
            Some(v) => serde_json::from_value(v)?,
            None => Ledger::default(),
        };
        ensure!(
            ledger.records.len() <= MAX_RECORDS,
            "发布记录超出上限，请在管理端检查"
        );
        Ok(ledger)
    }
    async fn save(&self, ledger: &Ledger) -> Result<()> {
        self.host
            .write_data(LEDGER, serde_json::to_value(ledger)?)
            .await
    }

    pub async fn publish(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        let mut post: QzonePost = serde_json::from_value(args)
            .map_err(|_| err(ToolErrorKind::InvalidArguments, "说说参数格式无效"))?;
        post.content = post.content.trim().to_owned();
        post.validate().map_err(|_| {
            err(
                ToolErrorKind::InvalidArguments,
                "说说正文最多2000字、图片最多9张，正文和图片不能同时为空",
            )
        })?;
        ensure!(
            post.images
                .iter()
                .all(|p| !p.contains("://") && !p.contains('\0') && p.len() <= 2048),
            err(
                ToolErrorKind::InvalidArguments,
                "图片只接受云团图库或插件输出的本地路径"
            )
        );
        let _guard = self.serial.lock().await;
        let status = self.host.qzone(ctx, QzoneAction::Status).await?;
        let qq = status["qq"]
            .as_u64()
            .filter(|q| *q > 0)
            .ok_or_else(|| err(ToolErrorKind::Unavailable, "QQ登录信息未就绪"))?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let day = (now + 28800) / 86400;
        let scope = ctx
            .task_id
            .clone()
            .unwrap_or_else(|| format!("command-day:{day}"));
        let input = serde_json::to_vec(&json!([qq, ctx.sender_pid, scope, post]))?;
        let key = format!("{:x}", Sha256::digest(input));
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&json!([qq, post]))?)
        );
        let mut ledger = self.load().await?;
        if let Some(old) = ledger.records.iter().find(|r| r.key == key) {
            match old.status.as_str() {
                "published" => {
                    let receipt = old
                        .receipt
                        .as_ref()
                        .filter(|v| {
                            v["status"] == "published"
                                && v["tid"].as_str().is_some_and(|t| !t.is_empty())
                        })
                        .ok_or_else(unknown)?;
                    return Ok(ToolOutput {
                        summary: format!(
                            "这条说说已发布，未重复发送。编号：{}",
                            receipt["tid"].as_str().unwrap()
                        ),
                        artifacts: vec![],
                        data: json!({"reused":true,"receipt":old.receipt,"request_id":key}),
                    });
                }
                "rejected" => {}
                _ => return Err(unknown()),
            }
        }
        if ledger.records.iter().any(|r| {
            r.fingerprint == fingerprint && matches!(r.status.as_str(), "sending" | "unknown")
        }) {
            return Err(unknown());
        }
        ledger.records.retain(|r| r.key != key);
        ensure!(
            ledger
                .records
                .iter()
                .filter(|r| r.day == day && r.status != "rejected")
                .count()
                < self.limit,
            err(
                ToolErrorKind::Unavailable,
                "已达到今日说说发布上限，待核实的尝试也计入额度"
            )
        );
        while ledger.records.len() >= MAX_RECORDS {
            let oldest = ledger
                .records
                .iter()
                .position(|r| r.day < day && matches!(r.status.as_str(), "published" | "rejected"));
            let Some(index) = oldest else {
                return Err(err(
                    ToolErrorKind::Unavailable,
                    "待核实发布记录过多，请先检查QQ空间与管理记录",
                ));
            };
            ledger.records.remove(index);
        }
        ledger.records.push(Record {
            key: key.clone(),
            fingerprint,
            day,
            created_at: now,
            qq,
            preview: post.content.chars().take(80).collect(),
            image_count: post.images.len(),
            visibility: post.visibility,
            status: "sending".into(),
            receipt: None,
        });
        self.save(&ledger).await?; // A failed reservation must never reach the external service.
        let result = self
            .host
            .qzone(
                ctx,
                QzoneAction::Publish {
                    post,
                    account_qq: qq,
                },
            )
            .await;
        let record = ledger.records.last_mut().unwrap();
        match result {
            Ok(receipt) => {
                // A malformed success is uncertain, not permission to publish it again.
                if receipt["status"] != "published"
                    || receipt["tid"].as_str().is_none_or(str::is_empty)
                {
                    record.status = "unknown".into();
                    let _ = self.save(&ledger).await;
                    return Err(unknown());
                }
                record.status = "published".into();
                record.receipt = Some(receipt.clone());
                let saved = self.save(&ledger).await.is_ok();
                Ok(ToolOutput {
                    summary: if saved {
                        format!("说说已发布，编号：{}", receipt["tid"].as_str().unwrap())
                    } else {
                        format!(
                            "说说已发布，但本地记录保存失败。请保留编号：{}",
                            receipt["tid"].as_str().unwrap()
                        )
                    },
                    artifacts: vec![],
                    data: json!({"receipt":receipt,"request_id":key,"receipt_recorded":saved}),
                })
            }
            Err(error) => {
                let precondition = error.downcast_ref::<ToolError>().is_some_and(|e| {
                    matches!(
                        e.kind,
                        ToolErrorKind::InvalidArguments
                            | ToolErrorKind::PermissionDenied
                            | ToolErrorKind::Unavailable
                    )
                });
                record.status = if precondition { "rejected" } else { "unknown" }.into();
                let _ = self.save(&ledger).await;
                if precondition {
                    Err(error)
                } else {
                    Err(unknown())
                }
            }
        }
    }
    pub async fn status(&self, ctx: &ToolCtx) -> Result<ToolOutput> {
        let data = self.host.qzone(ctx, QzoneAction::Status).await?;
        Ok(ToolOutput {
            summary: format!(
                "NapCat 已连接，当前 QQ：{}",
                data["qq"].as_u64().unwrap_or(0)
            ),
            artifacts: vec![],
            data,
        })
    }
    pub async fn history(&self, ctx: &ToolCtx) -> Result<ToolOutput> {
        let status = self.host.qzone(ctx, QzoneAction::Status).await?;
        let _guard = self.serial.lock().await;
        let ledger = self.load().await?;
        let records: Vec<_> = ledger
            .records
            .into_iter()
            .rev()
            .filter(|r| r.qq == status["qq"].as_u64().unwrap_or(0))
            .take(10)
            .map(|mut r| {
                if r.status == "sending" {
                    r.status = "unknown".into();
                }
                if ctx.chat_type == "group" && r.visibility == QzoneVisibility::Private {
                    r.preview = "私密说说（正文隐藏）".into();
                }
                r
            })
            .collect();
        Ok(ToolOutput {
            summary: if records.is_empty() {
                "暂无说说发布记录".into()
            } else {
                let lines: Vec<_> = records
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let status = match r.status.as_str() {
                            "published" => "已发布",
                            "rejected" => "未执行",
                            _ => "待核实",
                        };
                        let id = r
                            .receipt
                            .as_ref()
                            .and_then(|v| v["tid"].as_str())
                            .map(str::to_owned)
                            .unwrap_or_else(|| r.key.chars().take(12).collect());
                        format!("{}. {}：{}\n编号：{}", i + 1, status, r.preview, id)
                    })
                    .collect();
                format!(
                    "最近{}条说说发布记录：\n{}",
                    records.len(),
                    lines.join("\n")
                )
            },
            artifacts: vec![],
            data: json!({"records":records}),
        })
    }
}
