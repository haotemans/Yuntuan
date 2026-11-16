use super::*;
use serde_json::json;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "quark-auth-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("scripts")).unwrap();
        Self(path)
    }
    fn script(&self) -> PathBuf {
        self.0.join("scripts/quark-drive.cjs")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn callback_is_parsed_locally_and_requires_one_unambiguous_code() {
    assert_eq!(
        authorization_code(" https://localhost/callback?code=a%2Bb%3D&state=opaque ").unwrap(),
        "a+b="
    );
    assert_eq!(
        authorization_code("https://pan.quark.cn/done?agent_auth_code=one-time").unwrap(),
        "one-time"
    );
    assert_eq!(
        authorization_code("bare_code-123").unwrap(),
        "bare_code-123"
    );
    for value in [
        "",
        "javascript:alert(1)",
        "https://localhost/?code=a&code=b",
        "https://localhost/?code=a&agent_auth_code=b",
        "https://localhost/?state=a",
        "https://user:secret@localhost/?code=a",
        "a\nb",
        "a b",
        "--token bad",
    ] {
        assert!(authorization_code(value).is_err(), "{value}");
    }
    assert!(authorization_code(&"a".repeat(9000)).is_err());
}
#[test]
fn only_official_https_authorization_links_leave_the_process() {
    assert_eq!(
        auth_link(
            "🔗 [前往授权]https://pan.quark.cn/oauth?client_id=official&page_code=opaque\nhelp"
        ),
        Some("https://pan.quark.cn/oauth?client_id=official&page_code=opaque".into())
    );
    for link in [
        "http://pan.quark.cn/?client_id=x",
        "https://quark.cn.evil.test/?client_id=x",
        "https://user:secret@pan.quark.cn/?client_id=x",
        "https://pan.quark.cn/help",
        "https://pan.quark.cn:8443/?client_id=x",
    ] {
        assert!(auth_link(link).is_none());
    }
}
#[tokio::test]
async fn official_protocol_handles_headless_nonzero_exit_paste_login_and_existing_account() {
    let fixture = Fixture::new();
    std::fs::write(fixture.script(),r#"
const fs=require('fs'); const args=process.argv.slice(2);
const result=(code,msg)=>{console.log(JSON.stringify({type:'result',code,msg}));process.exit(code===0?0:1)};
if(args[0]==='resolve-agent') result(0,'fixture runtime');
if(args[0]==='get-user-info') result(fs.existsSync('authorized')?0:-1408,'private account');
if(args.includes('--token')) {
  if(args[2]!=='one-time') result(-1408,'do not leak '+args[2]);
  fs.writeFileSync('authorized','fixture'); result(0,'saved');
}
if(fs.existsSync('authorized')) result(-118,'already authorized');
result(-123,'🔗 [前往授权]https://pan.quark.cn/oauth?client_id=official&page_code=opaque\nhelp');
"#).unwrap();
    let script = prepare(&fixture.0, fixture.script().to_str().unwrap())
        .await
        .unwrap();
    assert!(matches!(begin(&script).await.unwrap(), Login::Link(_)));
    let error = complete(&script, "private-code")
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("private-code"));
    complete(&script, "one-time").await.unwrap();
    assert!(matches!(begin(&script).await.unwrap(), Login::Authorized));
    assert!(prepare(&fixture.0, "/missing/quark-drive.cjs")
        .await
        .is_err());
}

#[tokio::test]
async fn unsupported_runtime_is_a_precondition_not_an_oauth_failure() {
    let fixture = Fixture::new();
    std::fs::write(fixture.script(),r#"
const fs=require('fs'),args=process.argv.slice(2);
if(args[0]==='login') fs.writeFileSync('login-started','bad');
console.log(JSON.stringify({type:'result',code:-104,action:'runtime',msg:'untrusted diagnostics with private-token'}));process.exit(1);
"#).unwrap();
    let error = match begin(&fixture.script()).await {
        Err(error) => error,
        Ok(_) => panic!("unsupported runtime accepted"),
    };
    assert!(error.downcast_ref::<UnsupportedRuntime>().is_some());
    assert!(!fixture.0.join("login-started").exists());
    assert!(!error.to_string().contains("private-token"));
    let error = super::super::process::run(&fixture.script(), &["get-user-info".into()])
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<UnsupportedRuntime>().is_some());
    let error = complete(&fixture.script(), "one-time").await.unwrap_err();
    assert!(error.downcast_ref::<UnsupportedRuntime>().is_some());
    let error = verify(&fixture.script()).await.unwrap_err();
    assert!(error.downcast_ref::<UnsupportedRuntime>().is_some());
}
fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
#[test]
fn archive_rejects_traversal_and_preserves_existing_files() {
    let fixture = Fixture::new();
    assert!(extract(&zip(&[("../escape", "bad")]), &fixture.0).is_err());
    assert!(extract(&zip(&[("scripts\\escape", "bad")]), &fixture.0).is_err());
    let bytes = zip(&[("scripts/quark-drive.cjs", "original")]);
    extract(&bytes, &fixture.0).unwrap();
    assert!(extract(
        &zip(&[("scripts/quark-drive.cjs", "replacement")]),
        &fixture.0
    )
    .is_err());
    assert_eq!(
        std::fs::read_to_string(fixture.script()).unwrap(),
        "original"
    );
    assert_eq!(
        result(format!("{}\n", json!({"type":"result","code":0})).as_bytes()).unwrap()["code"],
        0
    );
}
