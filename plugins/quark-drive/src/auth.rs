//! Administrator onboarding through the official CLI, never a chat/Agent tool.
use anyhow::{bail, ensure, Context, Result};
use reqwest::Url;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PACKAGE: &str =
    "https://pdds.quark.cn/download/stfile/eekkigineefiegmgo/quarkclouddrive-1.0.25.zip";
const SHA256: &str = "56c34e9a9313ae29d39475b4ff9da9e92fbfd59699bf5843e3382e54434fffb8";
const ZIP_LIMIT: usize = 8 * 1024 * 1024;

pub enum Login {
    Authorized,
    Link(String),
}

/// The official CLI rejected this runtime before any OAuth/account request.
#[derive(Debug)]
pub struct UnsupportedRuntime;
impl std::fmt::Display for UnsupportedRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("夸克官方工具不支持当前 Agent 运行环境（-104），尚未进入 OAuth 登录；需要接入受支持的工具运行服务，重复登录或更换授权码无效")
    }
}
impl std::error::Error for UnsupportedRuntime {}
pub(crate) fn check_runtime(result: &Value) -> Result<()> {
    if result["code"] == -104 {
        return Err(UnsupportedRuntime.into());
    }
    Ok(())
}

pub async fn preflight(script: &Path) -> Result<()> {
    let (success, bytes) = super::process::capture(script, &["resolve-agent".into()]).await?;
    // Official 1.0.25's diagnostic is a channel marker, not result NDJSON.
    if success
        && std::str::from_utf8(&bytes).ok().is_some_and(|s| {
            s.trim().strip_prefix("QK_AGENT_ID=").is_some_and(|id| {
                !id.is_empty()
                    && id.len() <= 64
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            })
        })
    {
        return Ok(());
    }
    let out = result(&bytes)?;
    check_runtime(&out)?;
    ensure!(
        success && out["code"] == 0,
        "夸克运行环境检查失败，尚未发起 OAuth 登录"
    );
    Ok(())
}

pub fn validate_script(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute() && path.file_name().is_some_and(|n| n == "quark-drive.cjs"),
        "请填写容器内官方脚本的绝对路径"
    );
    let path = path
        .canonicalize()
        .context("官方组件尚未准备好，请重新发起授权")?;
    ensure!(path.is_file(), "官方脚本不是文件");
    Ok(path)
}

/// A pinned release, no installer shell or system changes. Never overwrite an
/// existing installation/account directory, including custom configured paths.
pub async fn prepare(data_root: &Path, configured: &str) -> Result<PathBuf> {
    if !configured.trim().is_empty() {
        return validate_script(Path::new(configured));
    }
    let parent = data_root.join("connectors");
    tokio::fs::create_dir_all(&parent).await?;
    let parent = tokio::fs::canonicalize(parent).await?;
    let target = parent.join("quark");
    if tokio::fs::try_exists(&target).await? {
        ensure!(
            !tokio::fs::symlink_metadata(&target)
                .await?
                .file_type()
                .is_symlink(),
            "组件目录不能是符号链接"
        );
        return validate_script(&target.join("scripts/quark-drive.cjs"));
    }
    let mut response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()?
        .get(PACKAGE)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("无法下载夸克官方组件，请检查网络后重试"))?;
    ensure!(
        response.status().is_success(),
        "官方组件下载失败（HTTP {}）",
        response.status().as_u16()
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= ZIP_LIMIT as u64),
        "官方组件包过大"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("官方组件下载中断"))?
    {
        ensure!(bytes.len() + chunk.len() <= ZIP_LIMIT, "官方组件包过大");
        bytes.extend_from_slice(&chunk);
    }
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == SHA256,
        "官方组件校验失败，未安装；请等待兼容版本更新"
    );
    let stage = parent.join(format!(
        ".quark-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir(&stage)?;
        private_dir(&stage)?;
        let result = (|| {
            extract(&bytes, &stage)?;
            ensure!(
                stage.join("scripts/quark-drive.cjs").is_file(),
                "官方组件包缺少主脚本"
            );
            ensure!(!target.exists(), "组件已由其他操作准备好，请重试");
            std::fs::rename(&stage, &target)?;
            validate_script(&target.join("scripts/quark-drive.cjs"))
        })();
        if stage.exists() {
            let _ = std::fs::remove_dir_all(&stage);
        }
        result
    })
    .await?
}
fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn extract(bytes: &[u8], root: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= 100, "组件包文件过多");
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        ensure!(
            !entry.name().contains('\\')
                && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "组件包包含不安全路径"
        );
        let relative = entry.enclosed_name().context("组件包路径越界")?;
        let path = root.join(relative);
        ensure!(entry.size() <= ZIP_LIMIT as u64, "组件文件过大");
        total = total.checked_add(entry.size()).context("组件包过大")?;
        ensure!(total <= 32 * 1024 * 1024, "组件包解压后过大");
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut content = Vec::new();
        (&mut entry)
            .take(ZIP_LIMIT as u64 + 1)
            .read_to_end(&mut content)?;
        ensure!(content.len() <= ZIP_LIMIT, "组件文件过大");
        file.write_all(&content)?;
    }
    Ok(())
}
fn result(bytes: &[u8]) -> Result<Value> {
    let mut values = std::str::from_utf8(bytes)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v["type"] == "result");
    let value = values
        .next()
        .context("官方授权未返回结果，请重新打开授权入口")?;
    ensure!(values.next().is_none(), "官方授权返回了多个结果");
    Ok(value)
}
/// Keep only the official link, not stdout, help text, account data or credentials.
fn auth_link(message: &str) -> Option<String> {
    message.split("https://").skip(1).find_map(|tail| {
        let candidate = format!(
            "https://{}",
            tail.split(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '`'))
                .next()?
        );
        let url = Url::parse(&candidate).ok()?;
        let host = url.host_str()?;
        (url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && (host == "quark.cn" || host.ends_with(".quark.cn"))
            && url.query_pairs().any(|(k, _)| k == "client_id"))
        .then(|| url.to_string())
    })
}
pub async fn begin(script: &Path) -> Result<Login> {
    preflight(script).await?;
    let (_, bytes) = super::process::capture(script, &["login".into()]).await?;
    let out = result(&bytes)?;
    check_runtime(&out)?;
    match out["code"].as_i64() {
        Some(0 | -118) => {
            verify(script).await?;
            Ok(Login::Authorized)
        }
        _ => auth_link(out["msg"].as_str().unwrap_or(""))
            .map(Login::Link)
            .context("未取得官方授权链接，请检查网络、Node.js 或在高级配置中指定现有官方脚本"),
    }
}
pub fn authorization_code(input: &str) -> Result<String> {
    let input = input.trim();
    ensure!(
        !input.is_empty() && input.len() <= 8192 && !input.chars().any(char::is_control),
        "请回填有效的授权码或跳转链接"
    );
    let code = if input.contains("://") {
        let url = Url::parse(input).context("跳转链接格式无效")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none(),
            "跳转链接格式无效"
        );
        let codes: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| matches!(k.as_ref(), "code" | "agent_auth_code" | "agentAuthCode"))
            .map(|(_, v)| v.into_owned())
            .collect();
        ensure!(
            codes.len() == 1,
            "跳转链接须包含唯一的授权码，请复制授权完成页显示的代码"
        );
        codes[0].clone()
    } else {
        input.to_owned()
    };
    ensure!(
        !code.is_empty()
            && code.len() <= 4096
            && code
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~+/=".contains(&c)),
        "授权码格式无效"
    );
    Ok(code)
}
pub async fn complete(script: &Path, code: &str) -> Result<()> {
    let (_, bytes) =
        super::process::capture(script, &["login".into(), "--token".into(), code.into()]).await?;
    let out = result(&bytes)?;
    check_runtime(&out)?;
    ensure!(
        matches!(out["code"].as_i64(), Some(0 | -118)),
        "授权未完成或授权码已失效，请重新获取；不会自动重试"
    );
    verify(script).await
}
pub async fn verify(script: &Path) -> Result<()> {
    let (success, bytes) = super::process::capture(script, &["get-user-info".into()]).await?;
    let out = result(&bytes)?;
    check_runtime(&out)?;
    if !success || out["code"] != 0 {
        bail!("账号连接检查未通过，请重新完成官方授权");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
