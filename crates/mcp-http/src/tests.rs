use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn sse_requires_complete_matching_events_and_keeps_split_utf8() {
    let data = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":3,\"result\":{\"tools\":[]}}\n\n";
    assert_eq!(
        parse_sse(data, 3).unwrap().unwrap()["result"]["tools"],
        json!([])
    );
    assert!(parse_sse(&data[..data.len() - 1], 3).unwrap().is_none());
    assert!(parse_sse(data, 4).unwrap().is_none());
    assert!(parse_sse(b"data: \xe4\xb8", 3).unwrap().is_none());
    assert!(parse_sse(b": keepalive\n\n", 3).unwrap().is_none());
    assert!(decode(json!({"jsonrpc":"2.0","id":2,"result":{}}), false, 1).is_err());
    let error = decode(
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"private-token"}}),
        false,
        1,
    )
    .unwrap_err();
    assert!(!error.to_string().contains("private-token"));
}
#[test]
fn rejects_unsafe_endpoints_and_header_tokens() {
    for url in [
        "http://docs.qq.com/openapi/mcp",
        "https://x:y@docs.qq.com/openapi/mcp",
        "https://docs.qq.com/mcp?token=secret",
    ] {
        assert!(HttpMcpClient::new(url, "token").is_err());
    }
    assert!(HttpMcpClient::new("https://docs.qq.com/openapi/mcp", "").is_err());
    assert!(HttpMcpClient::new("https://docs.qq.com/openapi/mcp", "token\nother").is_err());
}
#[tokio::test]
async fn real_http_handshake_session_pagination_and_call_are_correlated() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for step in 0..5 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (header_end, length);
            loop {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    header_end = i + 4;
                    let head = String::from_utf8_lossy(&bytes[..i]).to_ascii_lowercase();
                    assert!(head.contains("authorization: fixture-token"));
                    if step > 0 {
                        assert!(head.contains("mcp-session-id: session-one"));
                    }
                    length = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length: ")
                                .and_then(|n| n.parse::<usize>().ok())
                        })
                        .unwrap();
                    break;
                }
            }
            while bytes.len() < header_end + length {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            if step == 1 {
                assert_eq!(request["method"], "notifications/initialized");
                assert!(request.get("id").is_none());
                stream
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
                continue;
            }
            let result = match step {
                0 => {
                    assert_eq!(request["method"], "initialize");
                    json!({"protocolVersion":VERSION})
                }
                2 => json!({"tools":[{"name":"get_content"}],"nextCursor":"page-two"}),
                3 => {
                    assert_eq!(request["params"]["cursor"], "page-two");
                    json!({"tools":[{"name":"search_space_file"}]})
                }
                _ => {
                    assert_eq!(request["method"], "tools/call");
                    assert_eq!(request["params"]["arguments"]["file_id"], "file-one");
                    json!({"content":[{"type":"text","text":"actual fixture text"}]})
                }
            };
            let reply = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
            let body = if step == 4 {
                format!("event: message\ndata: {reply}\n\n")
            } else {
                reply.to_string()
            };
            let ct = if step == 4 {
                "text/event-stream"
            } else {
                "application/json"
            };
            let headers=format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nMcp-Session-Id: session-one\r\nConnection: close\r\n\r\n",body.len());
            stream.write_all(headers.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(7) {
                stream.write_all(chunk).await.unwrap();
            }
        }
    });
    let mut client =
        HttpMcpClient::new("https://docs.qq.com/openapi/mcp", "fixture-token").unwrap();
    client.url = Url::parse(&format!("http://{addr}/mcp")).unwrap(); // test-only loopback; production requires HTTPS.
    assert_eq!(client.list_tools().await.unwrap().len(), 2);
    assert_eq!(
        client
            .call_tool("get_content", json!({"file_id":"file-one"}))
            .await
            .unwrap()["content"][0]["text"],
        "actual fixture text"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn chunked_response_is_bounded_before_collecting_an_unlimited_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        let mut request = Vec::new();
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            if let Some(i) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..i]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length: ")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap();
                if request.len() >= i + 4 + length {
                    break;
                }
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let chunk = vec![b'x'; 8192];
        for _ in 0..160 {
            if stream.write_all(&chunk).await.is_err() {
                break;
            }
        }
    });
    let mut client =
        HttpMcpClient::new("https://docs.qq.com/openapi/mcp", "fixture-token").unwrap();
    client.url = Url::parse(&format!("http://{addr}/mcp")).unwrap();
    let error = client
        .exchange("tools/list", json!({}), None, false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("1 MiB"));
    server.await.unwrap();
}
