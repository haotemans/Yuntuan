//! Routing visibility and actionable validation; no external services.
use serde_json::{json, Value};
use std::path::PathBuf;
use yuantuan_core::{
    db,
    db::{params, SqliteExt},
    tools::{discovery, executor, Registry, Tool, ToolCtx, ToolOutput},
};
struct Fixture(PathBuf);
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yt-tool-use-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let f = Self(dir);
        db::migrate(&mut db::connect(&f.db()).await.unwrap())
            .await
            .unwrap();
        f
    }
    fn db(&self) -> PathBuf {
        self.0.join("db.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Probe {
    name: &'static str,
    ready: bool,
}
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "搜索公开资料，返回带来源的结果"
    }
    fn available_in_chat(&self, _: &ToolCtx) -> bool {
        self.ready
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","required":["query"],"properties":{"query":{"type":"string","minLength":1}}})
    }
    async fn call(&self, _: &ToolCtx, _: Value) -> anyhow::Result<ToolOutput> {
        panic!("discovery must never execute")
    }
}
#[tokio::test]
async fn tool_only_chat_gets_current_scoped_capabilities_without_executing() {
    let f = Fixture::new().await;
    let r = Registry::new();
    for (name, ready) in [
        ("mcp:docs.search", true),
        ("disabled", true),
        ("private", true),
        ("offline", false),
    ] {
        r.register(Probe { name, ready });
        executor::register(&f.db(), r.get(name).unwrap().as_ref())
            .await
            .unwrap();
    }
    let mut c = db::connect(&f.db()).await.unwrap();
    c.execute_batch("UPDATE capability_catalog SET enabled=0 WHERE name='disabled'; UPDATE capability_catalog SET scopes='[\"private:42\"]' WHERE name='private'; INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_tester',1,1);").await.unwrap();
    c.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,at_me,ts) VALUES('42','group','p_tester','检索项目发布，保留来源',1,100)",params![]).await.unwrap();
    let m = yuantuan_core::event::MessageReceivedPayload {
        msg_id: c.last_insert_rowid().await.unwrap(),
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_tester".into(),
        text: "检索项目发布，保留来源".into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 100,
    };
    let mut snapshot = yuantuan_core::context_builder::capture_reply_snapshot(
        &f.db(),
        &m,
        m.msg_id,
        &Default::default(),
    )
    .await
    .unwrap();
    discovery::refresh(&f.db(), Some(&r), &mut snapshot, &m.sender_pid).await;
    assert_eq!(snapshot.tool_capabilities["total"], 1);
    assert_eq!(
        snapshot.tool_capabilities["tools"][0]["name"],
        "mcp:docs.search"
    );
    assert_eq!(
        snapshot.tool_capabilities["tools"][0]["required"][0],
        "query"
    );
    let chat = yuantuan_core::context_builder::render_bot_context_with_skills(
        &snapshot,
        yuantuan_core::state::MoodValue::Calm,
        &Default::default(),
        &[],
    )
    .unwrap();
    let user: Value = serde_json::from_str(&chat.user).unwrap();
    assert_eq!(user["capabilities"]["tools_available"], true);
    assert!(user["capabilities"]["available_skills"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        user["capabilities"]["task_tools"]["tools"][0]["name"],
        "mcp:docs.search"
    );
    assert_eq!(user["anchor"]["text"], m.text);
    assert!(discovery::shrink(&mut snapshot.tool_capabilities));
    assert_eq!(snapshot.tool_capabilities["omitted"], 1);
    let chat = yuantuan_core::context_builder::render_bot_context_with_skills(
        &snapshot,
        yuantuan_core::state::MoodValue::Calm,
        &Default::default(),
        &[],
    )
    .unwrap();
    let user: Value = serde_json::from_str(&chat.user).unwrap();
    assert_eq!(
        user["capabilities"]["tools_available"], true,
        "omitted hints do not mean tools unavailable"
    );
    let unknown = discovery::overview(
        &f.db(),
        None,
        &ToolCtx {
            task_id: None,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_tester".into(),
            locale: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(unknown["known"], false);
}

#[test]
fn validation_reports_exact_paths_without_echoing_supplied_values() {
    let schema = json!({"type":"object","additionalProperties":false,"required":["queries"],"properties":{"queries":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"object","required":["query"],"additionalProperties":false,"properties":{"query":{"type":"string","minLength":1,"maxLength":12},"count":{"type":"integer","minimum":1,"maximum":3}}}}}});
    for (args, path) in [
        (json!({}), "/queries"),
        (json!({"queries":[{}]}), "/queries/0/query"),
        (json!({"queries":[{"query":42}]}), "/queries/0/query"),
        (
            json!({"queries":[{"query":"ok","count":4}]}),
            "/queries/0/count",
        ),
        (json!({"queries":[]}), "/queries"),
        (
            json!({"queries":[{"query":"private-input-value"}]}),
            "/queries/0/query",
        ),
        (
            json!({"queries":[{"query":"ok","wrong":"private-input-value"}]}),
            "/queries/0/wrong",
        ),
    ] {
        let error = yuantuan_core::capabilities::validate(&schema, &args)
            .unwrap_err()
            .to_string();
        assert!(error.contains(path), "{error}");
        assert!(!error.contains("private-input-value"));
    }
    yuantuan_core::capabilities::validate(
        &schema,
        &json!({"queries":[{"query":"公开问题","count":2}]}),
    )
    .unwrap();
    yuantuan_core::capabilities::validate(&json!({"type":["string","null"]}), &Value::Null)
        .unwrap();
    let escaped = yuantuan_core::capabilities::validate(
        &json!({"type":"object","required":["a/b~c"]}),
        &json!({}),
    )
    .unwrap_err()
    .to_string();
    assert!(escaped.contains("/a~1b~0c"));
}
