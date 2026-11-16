use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "quark-worker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(p.join("quark-service/jobs")).unwrap();
        std::fs::create_dir_all(p.join("quark/scripts")).unwrap();
        std::fs::write(p.join("quark/scripts/quark-drive.cjs"), "").unwrap();
        Self(p)
    }
    fn script(&self) -> PathBuf {
        self.0.join("quark/scripts/quark-drive.cjs")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn worker(f: &Fixture, receipt: bool) -> (Worker, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        f.0.join("quark-service/connection.json"),
        json!({"kind":"yuantuan-quark-tools-v1","url":format!("http://127.0.0.1:{port}/")})
            .to_string(),
    )
    .unwrap();
    std::fs::write(f.0.join("quark-service/password"), "p".repeat(40)).unwrap();
    let jobs = f.0.join("quark-service/jobs");
    let task = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let (header_end, total) = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let len = header
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, end + 4 + len);
                }
            };
            while bytes.len() < total {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            let first = header.lines().next().unwrap();
            let result = if first.starts_with("POST /session ") {
                json!({"id":"ses_fixture"})
            } else if first.starts_with("POST /session/ses_fixture/shell ") {
                let body: Value = serde_json::from_slice(&bytes[header_end..total]).unwrap();
                let command = body["command"].as_str().unwrap();
                assert!(command.starts_with("node /opt/quark-tools/job.cjs "));
                assert!(!command.contains("private-code"));
                let id = command.split_whitespace().last().unwrap();
                let input = jobs.join(format!("{id}.input.json"));
                let job: Value = serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
                assert_eq!(job["args"][2], "private-code");
                std::fs::remove_file(input).unwrap();
                if receipt {
                    std::fs::write(jobs.join(format!("{id}.output.json")),json!({"version":1,"id":id,"success":false,"error":null,"stdout":"{\"type\":\"result\",\"code\":-104}\n"}).to_string()).unwrap();
                }
                json!({"info":{"tokens":{"input":0,"output":0}},"parts":[]})
            } else {
                assert!(first.starts_with("DELETE /session/ses_fixture "));
                json!(true)
            };
            let body = result.to_string();
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (Worker::discover(&f.script()).await.unwrap().unwrap(), task)
}
#[tokio::test]
async fn private_receipt_preserves_business_error_without_codes_in_http_history() {
    let f = Fixture::new();
    let (worker, server) = worker(&f, true).await;
    let (success, bytes) = worker
        .capture(
            &f.script(),
            &["login".into(), "--token".into(), "private-code".into()],
        )
        .await
        .unwrap();
    assert!(!success);
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["code"],
        -104
    );
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_dir(f.0.join("quark-service/jobs"))
            .unwrap()
            .count(),
        0
    );
}
#[tokio::test]
async fn completed_http_call_without_actual_cli_receipt_is_not_success() {
    let f = Fixture::new();
    let (worker, server) = worker(&f, false).await;
    assert!(worker
        .capture(
            &f.script(),
            &["login".into(), "--token".into(), "private-code".into()]
        )
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_dir(f.0.join("quark-service/jobs"))
            .unwrap()
            .count(),
        0
    );
}
