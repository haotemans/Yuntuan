//! Bounded child-process and NDJSON handling. No shell, unbounded output collection or write retries.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

async fn bounded<R: AsyncRead + Unpin>(reader: R, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes).await?;
    ensure!(bytes.len() as u64 <= limit, "夸克 CLI 输出超过上限");
    Ok(bytes)
}
pub(super) async fn run(script: &Path, args: &[String]) -> Result<Value> {
    let root = script
        .parent()
        .and_then(Path::parent)
        .context("官方 CLI 目录无效")?;
    let mut child = Command::new("node")
        .arg(script)
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("启动夸克 CLI 失败，请检查容器内 Node.js 与脚本路径")?;
    let stdout = child.stdout.take().context("CLI stdout 未就绪")?;
    let stderr = child.stderr.take().context("CLI stderr 未就绪")?;
    let work = async {
        let (out, _err, status) =
            tokio::try_join!(bounded(stdout, 256_000), bounded(stderr, 16_000), async {
                child.wait().await.map_err(anyhow::Error::from)
            })?;
        ensure!(
            status.success(),
            "夸克 CLI 非正常退出，请检查官方安装和授权"
        );
        parse_output(root, &out).await
    };
    tokio::time::timeout(Duration::from_secs(100), work)
        .await
        .context("夸克 CLI 执行超时")?
}
pub(super) async fn parse_output(root: &Path, bytes: &[u8]) -> Result<Value> {
    let stdout = std::str::from_utf8(bytes).context("CLI 返回非 UTF-8 内容")?;
    let mut result = None;
    let mut rows = vec![];
    let mut artifacts = vec![];
    for line in stdout.lines().filter(|s| !s.trim().is_empty()) {
        // Other CLI diagnostics are not forwarded to the model or logs.
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            match value["type"].as_str() {
                Some("result") => {
                    ensure!(result.is_none(), "CLI 返回了多条最终结果");
                    result = Some(value);
                }
                Some("list") | Some("file") => rows.push(value["data"].clone()),
                Some("artifact") => {
                    if let Some(path) = value["data"]["file_path"].as_str() {
                        artifacts.push(path.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    let mut result = result.context("夸克 CLI 未返回业务结果，请检查安装与账号授权")?;
    let code = result["code"]
        .as_i64()
        .context("CLI 业务结果缺少有效状态码")?;
    let notice = result["notice"]["text"].as_str().unwrap_or("");
    ensure!(
        notice.len() <= 8000,
        "CLI notice 过长，未展示完整提示，请检查官方 CLI"
    );
    if code != 0 {
        let message = format!(
            "夸克接口返回错误代码 {code}，请检查授权或链接状态。{}{}",
            if notice.is_empty() { "" } else { "\n\n" },
            notice
        );
        anyhow::bail!(message);
    }
    let mut truncated = false;
    for path in artifacts.iter().take(2) {
        let supplied = Path::new(path);
        let absolute = if supplied.is_absolute() {
            supplied.to_owned()
        } else {
            root.join(supplied)
        };
        let file = tokio::fs::canonicalize(absolute)
            .await
            .context("CLI 结果文件不存在")?;
        let canonical_root = tokio::fs::canonicalize(root).await?;
        ensure!(
            file.starts_with(&canonical_root) && file.extension().is_some_and(|e| e == "jsonl"),
            "CLI 结果文件不在官方安装目录内"
        );
        let mut bytes = Vec::new();
        tokio::fs::File::open(file)
            .await?
            .take(256_001)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 256_000 {
            truncated = true;
            let end = bytes[..256_000]
                .iter()
                .rposition(|b| *b == b'\n')
                .context("CLI 结果单行过长")?;
            bytes.truncate(end);
        }
        let text = std::str::from_utf8(&bytes).context("CLI 结果文件非 UTF-8")?;
        for line in text.lines().filter(|s| !s.trim().is_empty()) {
            let row: Value = serde_json::from_str(line).context("CLI JSONL 结果格式无效")?;
            if rows.len() >= 50 {
                truncated = true;
                break;
            }
            rows.push(row);
        }
    }
    if artifacts.len() > 2 {
        truncated = true;
    }
    while serde_json::to_vec(&rows)?.len() > 24_000 {
        rows.pop();
        truncated = true;
    }
    result["items"] = Value::Array(rows);
    strip_credentials(&mut result);
    result["items_truncated"] = Value::Bool(truncated);
    ensure!(
        serde_json::to_vec(&result)?.len() <= 48_000,
        "夸克结果过大，请缩小查询范围"
    );
    Ok(result)
}

fn strip_credentials(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| {
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "token"
                        | "access_token"
                        | "refresh_token"
                        | "cookie"
                        | "authorization"
                        | "stoken"
                )
            });
            for v in map.values_mut() {
                strip_credentials(v);
            }
        }
        Value::Array(a) => {
            for v in a {
                strip_credentials(v);
            }
        }
        _ => {}
    }
}
