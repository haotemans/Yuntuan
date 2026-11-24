//! Bounded runtime log cache and nonblocking, rotating JSONL persistence.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::{atomic::{AtomicU64, AtomicUsize, Ordering}, mpsc, Arc, Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 2000;
const FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: u64,
    pub ts: u64,
    pub level: String,
    pub target: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub fields: BTreeMap<String, String>,
}

enum WriteCommand {
    Entry(LogEntry),
    Flush(mpsc::Sender<()>),
}

#[derive(Default)]
struct State {
    entries: VecDeque<LogEntry>,
    writer: Option<mpsc::SyncSender<WriteCommand>>,
}

#[derive(Clone, Default)]
pub struct RuntimeLogs {
    state: Arc<Mutex<State>>,
    next_id: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    disk_errors: Arc<AtomicU64>,
    level: Arc<AtomicUsize>,
    secrets: Arc<RwLock<Vec<String>>>,
}

fn rank(level: &str) -> usize {
    match level { "TRACE" => 1, "DEBUG" => 2, "INFO" => 3, "WARN" => 4, "ERROR" => 5, _ => 3 }
}

impl RuntimeLogs {
    pub fn set_level(&self, level: &str) {
        self.level.store(rank(&level.to_uppercase()), Ordering::Relaxed);
    }

    pub fn add_secret(&self, secret: &str) {
        if secret.len() >= 4 {
            let mut values = self.secrets.write().unwrap();
            if !values.iter().any(|value| value == secret) { values.push(secret.into()); }
            values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        }
    }

    pub fn register_secrets(&self, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(values) => for (key, value) in values {
                let key = key.to_ascii_lowercase();
                if ["password", "secret", "token", "cookie", "api_key", "authorization"].iter().any(|word|key.contains(word)) {
                    if let Some(secret) = value.as_str() {
                        self.add_secret(secret);
                        if key == "api_key_env" {
                            if let Some(resolved) = crate::llm::resolve_api_key(secret) { self.add_secret(&resolved); }
                        }
                    }
                }
                self.register_secrets(value);
            },
            serde_json::Value::Array(values) => for value in values { self.register_secrets(value); },
            _ => {},
        }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in self.secrets.read().unwrap().iter() { text = text.replace(secret, "[REDACTED]"); }
        for key in ["api_key", "appsecret", "access_token", "password", "authorization", "cookie", "token", "secret"] {
            let mut cursor = 0;
            while let Some(offset) = text[cursor..].to_ascii_lowercase().find(key) {
                let end_key = cursor + offset + key.len();
                let after = &text[end_key..];
                let stripped = after.trim_start_matches(['"','\'',' ', '\t']);
                let Some(value) = stripped.strip_prefix([':', '=']) else { cursor=end_key; continue };
                let value = value.trim_start_matches([' ', '\t','"','\'']);
                let start = text.len() - value.len();
                let end = value.find(|c: char| c.is_whitespace() || ['"','\'',',','}',']'].contains(&c)).map(|n|start+n).unwrap_or(text.len());
                if start<end {
                    text.replace_range(start..end,"[REDACTED]");
                    cursor=start+"[REDACTED]".len();
                } else { cursor=end_key; }
            }
        }
        // URLs may carry credentials in paths, queries or user-info.
        for prefix in ["https://", "http://", "wss://", "ws://", "Bearer ", "bearer ", "sk-"] {
            let mut start = 0;
            while let Some(position) = text[start..].find(prefix) {
                let position = start + position;
                let end = text[position..].find(|c: char| c.is_whitespace() || ['"', '\'', '<', '>'].contains(&c))
                    .map(|offset| position + offset).unwrap_or(text.len());
                // For Bearer, consume the following credential as well.
                let end = if prefix.ends_with(' ') {
                    text[position + prefix.len()..].find(|c: char| c.is_whitespace() || ['"', '\''].contains(&c))
                        .map(|offset| position + prefix.len() + offset).unwrap_or(text.len())
                } else { end };
                text.replace_range(position..end, "[REDACTED]");
                start = position + "[REDACTED]".len();
            }
        }
        text.chars().take(8192).collect()
    }

    pub fn record(&self, level: &str, target: &str, message: &str, file: Option<&str>, line: Option<u32>, fields: BTreeMap<String, String>) {
        let configured = self.level.load(Ordering::Relaxed);
        let minimum = if configured == 0 { 3 } else { configured };
        if rank(level) < minimum { return; }
        let micros = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_micros() as u64;
        self.next_id.fetch_max(micros, Ordering::Relaxed);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut budget = 12_288;
        let fields = fields.into_iter().take(24).map(|(key, value)| {
            let name = key.to_lowercase();
            let value = if ["token", "password", "secret", "cookie", "authorization", "api_key"].iter().any(|s| name.contains(s)) {
                "[REDACTED]".into()
            } else { self.redact(&value) };
            let value: String = value.chars().take(budget).collect();
            budget = budget.saturating_sub(value.chars().count());
            (key, value)
        }).collect();
        let entry = LogEntry {
            id, ts: micros / 1000, level: level.into(), target: target.chars().take(160).collect(),
            message: self.redact(message), file: file.map(str::to_owned), line,
            fields,
        };
        let mut state = self.state.lock().unwrap();
        if let Some(writer) = &state.writer {
            if writer.try_send(WriteCommand::Entry(entry.clone())).is_err() { self.dropped.fetch_add(1, Ordering::Relaxed); }
        }
        state.entries.push_back(entry);
        while state.entries.len() > CAPACITY { state.entries.pop_front(); }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.state.lock().unwrap().entries.iter().cloned().map(|mut entry| {
            entry.message = self.redact(&entry.message);
            for value in entry.fields.values_mut() { *value = self.redact(value); }
            entry
        }).collect()
    }

    pub fn status(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap();
        serde_json::json!({"capacity":CAPACITY,"cached":state.entries.len(),"persisting":state.writer.is_some(),
            "dropped_writes":self.dropped.load(Ordering::Relaxed),"disk_errors":self.disk_errors.load(Ordering::Relaxed),
            "minimum_level":match self.level.load(Ordering::Relaxed) {1=>"TRACE",2=>"DEBUG",4=>"WARN",5=>"ERROR",_=>"INFO" },
            "file_limit_bytes":FILE_BYTES,"backup_files":3})
    }

    pub fn start_file(&self, directory: &Path) -> std::io::Result<()> {
        fs::create_dir_all(directory)?;
        let path = directory.join("runtime.jsonl");
        let mut state = self.state.lock().unwrap();
        if state.writer.is_some() { return Ok(()); }
        let mut history = VecDeque::new();
        for suffix in [".3", ".2", ".1", ""] {
            let file = match fs::File::open(format!("{}{suffix}", path.display())) { Ok(file) => file, Err(_) => continue };
            if file.metadata()?.len() > FILE_BYTES + 128 * 1024 { continue; }
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                    history.push_back(entry);
                    if history.len() > CAPACITY { history.pop_front(); }
                }
            }
        }
        history.extend(state.entries.iter().cloned());
        while history.len() > CAPACITY { history.pop_front(); }
        let file = open_log(&path)?;
        let (tx, rx) = mpsc::sync_channel(512);
        let errors = self.disk_errors.clone();
        std::thread::Builder::new().name("runtime-log-writer".into()).spawn(move || {
            let mut file = Some(file);
            while let Ok(command) = rx.recv() {
                match command {
                    WriteCommand::Flush(done) => { if let Some(file) = &mut file { let _ = file.flush(); } let _ = done.send(()); }
                    WriteCommand::Entry(entry) => {
                        let result = (|| -> std::io::Result<()> {
                            if file.as_ref().unwrap().metadata()?.len() >= FILE_BYTES {
                                drop(file.take());
                                rotate(&path)?;
                                file = Some(open_log(&path)?);
                            }
                            let file = file.as_mut().unwrap();
                            serde_json::to_writer(&mut *file, &entry)?;
                            file.write_all(b"\n")
                        })();
                        if result.is_err() { errors.fetch_add(1, Ordering::Relaxed); break; }
                    }
                }
            }
        })?;
        for entry in &state.entries {
            if tx.try_send(WriteCommand::Entry(entry.clone())).is_err() { self.dropped.fetch_add(1, Ordering::Relaxed); }
        }
        state.entries = history;
        state.writer = Some(tx);
        Ok(())
    }

    pub fn flush(&self) {
        let writer = self.state.lock().unwrap().writer.clone();
        if let Some(writer) = writer {
            let (done, wait) = mpsc::channel();
            if writer.try_send(WriteCommand::Flush(done)).is_ok() { let _ = wait.recv_timeout(Duration::from_secs(2)); }
        }
    }
}

fn open_log(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    options.open(path)
}

fn rotate(path: &Path) -> std::io::Result<()> {
    let name = path.to_string_lossy();
    match fs::remove_file(format!("{name}.3")) { Ok(()) => {}, Err(e) if e.kind()==std::io::ErrorKind::NotFound => {}, Err(e)=>return Err(e) }
    for (from, to) in [(format!("{name}.2"),format!("{name}.3")),(format!("{name}.1"),format!("{name}.2")),(name.to_string(),format!("{name}.1"))] {
        match fs::rename(from,to) { Ok(())=>{}, Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{}, Err(e)=>return Err(e) }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_before_persistence_and_restores_bounded_history() {
        let root = std::env::temp_dir().join(format!("yt-logs-{}",rand::random::<u64>()));
        let logs = RuntimeLogs::default();
        logs.register_secrets(&serde_json::json!({"provider":{"api_key_env":"sk-private-key"},"app_secret":"private-secret"}));
        let boundary=format!("{}private-secret","x".repeat(8188));
        assert!(!logs.redact(&boundary).contains("priv"));
        logs.record("ERROR","model","HTTP 401 private-secret https://host/private?token=abc Bearer hidden-value",Some("test.rs"),Some(9),
            BTreeMap::from([("error".into(),"{\"token\":\"dynamic-value\"} sk-private-key".into()),("password".into(),"hidden-pass".into())]));
        logs.start_file(&root).unwrap();
        logs.flush();
        let disk = fs::read_to_string(root.join("runtime.jsonl")).unwrap();
        for secret in ["private-secret","host/private","hidden-value","dynamic-value","sk-private-key","hidden-pass"] {
            assert!(!disk.contains(secret),"leaked {secret}");
        }
        assert!(disk.contains("HTTP 401"));
        let restored = RuntimeLogs::default();
        restored.start_file(&root).unwrap();
        assert_eq!(restored.entries().len(),1);
        assert_eq!(restored.entries()[0].line,Some(9));
        for _ in 0..CAPACITY+5 { restored.record("INFO","test","message",None,None,BTreeMap::new()); }
        let entries = restored.entries();
        assert_eq!(entries.len(),CAPACITY);
        assert!(entries.windows(2).all(|pair|pair[0].id<pair[1].id));
        restored.flush();
        drop(restored);
        drop(logs);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn log_rotation_preserves_current_and_three_backups() {
        let root = std::env::temp_dir().join(format!("yt-log-rotate-{}",rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("runtime.jsonl");
        for n in 0..6 { fs::write(&path,n.to_string()).unwrap(); rotate(&path).unwrap(); }
        assert_eq!(fs::read_to_string(root.join("runtime.jsonl.1")).unwrap(),"5");
        assert_eq!(fs::read_to_string(root.join("runtime.jsonl.3")).unwrap(),"3");
        assert!(!root.join("runtime.jsonl.4").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
