CREATE TABLE IF NOT EXISTS working_context_versions (
  context_id INTEGER NOT NULL REFERENCES messages(msg_id),
  revision INTEGER NOT NULL,
  chat_id TEXT NOT NULL,
  chat_type TEXT NOT NULL CHECK(chat_type IN ('group','private')),
  owner_pid TEXT NOT NULL,
  source_msg_id INTEGER NOT NULL REFERENCES messages(msg_id),
  source_ts INTEGER NOT NULL,
  goal_quote TEXT NOT NULL,
  goal_source_id INTEGER NOT NULL REFERENCES messages(msg_id),
  conditions TEXT NOT NULL DEFAULT '[]',
  recorded_at INTEGER NOT NULL,
  PRIMARY KEY(context_id,revision),
  UNIQUE(context_id,source_msg_id)
);
CREATE INDEX IF NOT EXISTS idx_working_scope ON working_context_versions(chat_id,chat_type,owner_pid,source_msg_id);
