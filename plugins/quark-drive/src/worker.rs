//! Private OpenCode tool worker: native shell execution without a model turn.
//! Only random tickets enter shell history; codes and CLI output use transient
//! private files in the shared Quark directory, outside the worker transcript.
use anyhow::{ensure, Context, Result};
use rand::{distr::Alphanumeric, Rng};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LIMIT: usize = 2 * 1024 * 1024;
pub(super) struct Worker {
    root: PathBuf,
    url: Url,
    password: String,
    http: Client,
}
impl Worker {
    pub async fn discover(script: &Path) -> Result<Option<Self>> {
        for root in script.ancestors().skip(1).take(8) {
            let config = root.join("quark-service/connection.json");
            if !tokio::fs::try_exists(&config).await? {
                continue;
            }
            let mut bytes = Vec::new();
            tokio::fs::File::open(&config)
                .await?
                .take(8193)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= 8192, "夸克运行服务配置过大");
            let value: Value = serde_json::from_slice(&bytes).context("夸克运行服务配置无效")?;
            ensure!(
                value["kind"] == "yuantuan-quark-tools-v1",
                "夸克运行服务配置版本不支持"
            );
            let url = Url::parse(value["url"].as_str().context("缺少夸克运行服务地址")?)
                .context("夸克运行服务地址无效")?;
            ensure!(
                url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path() == "/"
                    && url.scheme() == "http"
                    && matches!(
                        url.host_str(),
                        Some("quark-tools" | "localhost" | "127.0.0.1" | "[::1]")
                    ),
                "夸克运行服务须使用指定的内网地址"
            );
            let password = tokio::fs::read_to_string(root.join("quark-service/password"))
                .await
                .context("夸克运行服务凭据未准备好")?;
            ensure!(
                (32..=256).contains(&password.trim().len())
                    && !password.trim().chars().any(char::is_control),
                "夸克运行服务凭据无效"
            );
            return Ok(Some(Self {
                root: root.to_owned(),
                url,
                password: password.trim().into(),
                http: Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(Duration::from_secs(110))
                    .build()?,
            }));
        }
        Ok(None)
    }
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self.url.join(path)?;
        let mut req = self
            .http
            .request(method, url)
            .basic_auth("opencode", Some(&self.password));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let mut res = req
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("夸克工具运行服务无法连接，请检查组件状态"))?;
        ensure!(
            res.status().is_success(),
            "夸克工具运行服务返回 HTTP {}，请检查组件及授权配置",
            res.status().as_u16()
        );
        ensure!(
            res.content_length().is_none_or(|n| n <= LIMIT as u64),
            "夸克工具服务回执过大"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("夸克工具服务响应中断"))?
        {
            ensure!(bytes.len() + chunk.len() <= LIMIT, "夸克工具服务回执过大");
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("夸克工具服务返回无效回执")
    }
    pub async fn capture(&self, script: &Path, args: &[String]) -> Result<(bool, Vec<u8>)> {
        static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _serial = tokio::time::timeout(Duration::from_secs(120), SERIAL.lock())
            .await
            .context("夸克工具服务繁忙，请稍后重试")?;
        let id: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        let jobs = self.root.join("quark-service/jobs");
        tokio::fs::create_dir_all(&jobs).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&jobs, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let files = JobFiles {
            input: jobs.join(format!("{id}.input.json")),
            output: jobs.join(format!("{id}.output.json")),
            active: jobs.join(format!("{id}.active")),
        };
        let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 120;
        write_private(&files.active, b"active").await?;
        let input = serde_json::to_vec(
            &json!({"version":1,"id":id,"script":script,"args":args,"deadline":deadline}),
        )?;
        ensure!(input.len() <= 65536, "夸克工具请求过大");
        write_private(&files.input, &input).await?;
        let created = self
            .request(
                reqwest::Method::POST,
                "session",
                Some(json!({"title":"Quark tool invocation"})),
            )
            .await?;
        let session = created["id"]
            .as_str()
            .filter(|s| {
                s.starts_with("ses")
                    && s.len() <= 128
                    && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            })
            .context("夸克工具服务未创建有效执行会话")?
            .to_owned();
        let close = SessionCleanup {
            http: self.http.clone(),
            url: self.url.join(&format!("session/{session}"))?,
            password: self.password.clone(),
        };
        let command = format!("node /opt/quark-tools/job.cjs {id}");
        self.request(
            reqwest::Method::POST,
            &format!("session/{session}/shell"),
            Some(json!({"agent":"build","command":command})),
        )
        .await?;
        // A generic completed tool is insufficient: require this exact ticket's
        // bounded local receipt written after the official CLI actually exits.
        let mut bytes = Vec::new();
        tokio::fs::File::open(&files.output)
            .await
            .context("夸克工具未返回实际执行结果")?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= LIMIT, "夸克实际执行结果过大");
        let result: Value = serde_json::from_slice(&bytes).context("夸克执行结果格式无效")?;
        ensure!(
            result["version"] == 1 && result["id"] == id,
            "夸克执行结果与当前请求不匹配"
        );
        ensure!(
            result["error"].is_null(),
            "夸克工具执行未完成，请检查运行服务；写操作结果需核实，不会自动重试"
        );
        let stdout = result["stdout"]
            .as_str()
            .filter(|s| s.len() <= 256000)
            .context("夸克工具未返回有效输出")?
            .as_bytes()
            .to_vec();
        let success = result["success"]
            .as_bool()
            .context("夸克执行结果缺少状态")?;
        drop(close);
        drop(files);
        Ok((success, stdout))
    }
}
async fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
}
struct JobFiles {
    input: PathBuf,
    output: PathBuf,
    active: PathBuf,
}
impl Drop for JobFiles {
    fn drop(&mut self) {
        for path in [&self.active, &self.input, &self.output] {
            let _ = std::fs::remove_file(path);
        }
    }
}
struct SessionCleanup {
    http: Client,
    url: Url,
    password: String,
}
impl Drop for SessionCleanup {
    fn drop(&mut self) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let (http, url, password) =
                (self.http.clone(), self.url.clone(), self.password.clone());
            runtime.spawn(async move {
                let _ = http
                    .delete(url)
                    .basic_auth("opencode", Some(password))
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await;
            });
        }
    }
}

#[cfg(test)]
mod tests;
