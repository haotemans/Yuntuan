use super::*;
use std::sync::atomic::AtomicBool;
struct Host {
    disabled: AtomicBool,
    choice: Mutex<Value>,
    checked: Mutex<Vec<String>>,
}
#[async_trait]
impl HostServices for Host {
    fn account_owner_qq(&self) -> Option<u64> {
        Some(123)
    }
    async fn authorize_tool_for_skill(&self, _: &ToolCtx, name: &str, args: &Value) -> Result<()> {
        self.checked.lock().unwrap().push(name.into());
        if self.disabled.load(Ordering::SeqCst) {
            bail!("Tool disabled");
        }
        validate_tool_arguments(
            &Op::from_tool_name(name).context("unknown tool")?.schema(),
            args,
        )
    }
    async fn chat(&self, _: &ToolCtx, _: &str, _: &str) -> Result<String> {
        Ok(self.choice.lock().unwrap().to_string())
    }
    async fn read_data(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn write_data(&self, _: &str, _: Value) -> Result<()> {
        Ok(())
    }
    fn log(&self, _: &str) {}
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: Some("test".into()),
        chat_id: "42".into(),
        chat_type: "private".into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}
fn root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "quark-fixture-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn links_schemas_and_write_permissions_are_explicit() {
    for bad in [
        "https://evil.test/s/abc",
        "https://pan.quark.cn/s/",
        "https://pan.quark.cn/s/abc#frag",
        "https://pan.quark.cn/s/../../secret",
    ] {
        assert!(check_share_url(bad).is_err());
    }
    assert!(check_share_url("https://pan.quark.cn/s/abc123?pwd=a123").is_ok());
    assert!(Op::from_tool_name("quark-drive.delete").is_none());
    assert!(
        validate_tool_arguments(&Op::Search.schema(), &json!({"keyword":"a","shell":"bad"}))
            .is_err()
    );
    assert!(
        validate_tool_arguments(&Op::Search.schema(), &json!({"keyword":"长".repeat(51)})).is_err()
    );
}
#[tokio::test]
async fn ndjson_keeps_all_list_rows_artifact_preview_and_final_notice_without_credentials() {
    let root = root();
    tokio::fs::create_dir_all(root.join("search"))
        .await
        .unwrap();
    tokio::fs::write(
        root.join("search/items.jsonl"),
        b"{\"fid\":\"file-two\",\"name\":\"from artifact\"}\n",
    )
    .await
    .unwrap();
    let output=b"{\"type\":\"list\",\"data\":{\"fid\":\"file-one\",\"name\":\"row\",\"access_token\":\"private\"}}\n{\"type\":\"result\",\"code\":0,\"data\":{\"count\":2},\"notice\":{\"text\":\"Official notice\"}}\n{\"type\":\"artifact\",\"data\":{\"file_path\":\"search/items.jsonl\"}}\n";
    let data = process::parse_output(&root, output).await.unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 2);
    assert_eq!(data["items"][1]["fid"], "file-two");
    assert!(!data.to_string().contains("private"));
    let out = render_output(Op::Search, data);
    assert!(out.summary.contains("Official notice"));
    assert!(out.summary.contains("file-one"));
    assert!(process::parse_output(
        &root,
        b"{\"type\":\"result\",\"code\":-403,\"notice\":{\"text\":\"failure notice\"}}\n"
    )
    .await
    .unwrap_err()
    .to_string()
    .contains("failure notice"));
    assert!(process::parse_output(&root,b"{\"type\":\"result\",\"code\":0}\n{\"type\":\"artifact\",\"data\":{\"file_path\":\"../outside.jsonl\"}}\n").await.is_err());
    tokio::fs::remove_dir_all(root).await.unwrap();
}
#[tokio::test]
async fn actual_node_argv_is_guarded_and_session_is_reused_without_shell_expansion() {
    let root = root();
    tokio::fs::create_dir_all(root.join("scripts"))
        .await
        .unwrap();
    let cli = root.join("scripts/quark-drive.cjs");
    tokio::fs::write(&cli,r#"const fs=require('fs');fs.appendFileSync('calls.jsonl',JSON.stringify(process.argv.slice(2))+'\n');console.log(JSON.stringify({type:'list',data:{fid:'fixture-fid',name:'actual file'}}));console.log(JSON.stringify({type:'result',code:0,data:{count:1},notice:{text:'Official fixture notice'}}));"#).await.unwrap();
    let host = Arc::new(Host {
        disabled: AtomicBool::new(true),
        choice: Mutex::new(
            json!({"name":"quark-drive.search","arguments":{"keyword":"a;$(echo nope)"}}),
        ),
        checked: Mutex::new(vec![]),
    });
    let caps = plugin()
        .start(
            PluginContext { host: host.clone() },
            json!({"cli_script":cli.to_string_lossy()}),
        )
        .await
        .unwrap();
    let request = json!({"user_text":"搜索 a;$(echo nope)"});
    assert!(caps.skills[0]
        .invoke(&ctx(), request.clone())
        .await
        .is_err());
    assert!(!root.join("calls.jsonl").exists());
    host.disabled.store(false, Ordering::SeqCst);
    let other = ToolCtx {
        sender_pid: "p_456".into(),
        ..ctx()
    };
    assert!(caps.tools[0].call(&other, json!({})).await.is_err());
    assert!(!root.join("calls.jsonl").exists());
    let out = caps.skills[0].invoke(&ctx(), request).await.unwrap();
    assert!(out.summary.contains("fixture-fid"));
    assert!(out.summary.contains("Official fixture notice"));
    assert_eq!(
        host.checked.lock().unwrap().last().unwrap(),
        "quark-drive.search"
    );
    caps.tools[0].call(&ctx(), json!({})).await.unwrap();
    let calls = tokio::fs::read_to_string(root.join("calls.jsonl"))
        .await
        .unwrap();
    let calls: Vec<Value> = calls
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0][0], "search");
    assert_eq!(calls[0][2], "a;$(echo nope)");
    let a = calls[0].as_array().unwrap();
    let b = calls[1].as_array().unwrap();
    let i = a.iter().position(|v| v == "--session-id").unwrap();
    let j = b.iter().position(|v| v == "--session-id").unwrap();
    assert_eq!(a[i + 1], b[j + 1]);
    assert!(caps.tools[5]
        .call(&ctx(), json!({"fids":["known-fid"]}))
        .await
        .is_err());
    assert_eq!(
        tokio::fs::read_to_string(root.join("calls.jsonl"))
            .await
            .unwrap()
            .lines()
            .count(),
        2
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}
