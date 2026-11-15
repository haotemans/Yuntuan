-- Exact message coverage allows recent work without pretending an old backlog is complete.
CREATE TABLE IF NOT EXISTS portrait_coverage (
  person_id TEXT NOT NULL REFERENCES persons(person_id),
  msg_id INTEGER NOT NULL REFERENCES messages(msg_id) ON DELETE CASCADE,
  batch_id INTEGER NOT NULL REFERENCES memory_batches(id),
  processed_at INTEGER NOT NULL,
  PRIMARY KEY(person_id,msg_id)
);
CREATE TABLE IF NOT EXISTS portrait_runs (
  batch_id INTEGER PRIMARY KEY REFERENCES memory_batches(id),
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  reasoning TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'running',
  error_kind TEXT,
  elapsed_ms INTEGER NOT NULL DEFAULT 0,
  repairs INTEGER NOT NULL DEFAULT 0,
  inspected_messages INTEGER NOT NULL DEFAULT 0,
  updated_fields INTEGER NOT NULL DEFAULT 0,
  rejected_updates INTEGER NOT NULL DEFAULT 0,
  source_start_ts INTEGER,
  source_end_ts INTEGER
);
CREATE INDEX IF NOT EXISTS idx_portrait_runs_status ON portrait_runs(status,batch_id);
CREATE INDEX IF NOT EXISTS idx_msg_sender_id ON messages(sender_pid,msg_id);
