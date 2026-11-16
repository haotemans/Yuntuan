-- Shared daily read-only tool scheduler; each job is owned by the local administrator.
CREATE TABLE IF NOT EXISTS scheduled_tools (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  resource_key TEXT NOT NULL,
  arguments_json TEXT NOT NULL,
  timezone_offset_minutes INTEGER NOT NULL DEFAULT 480,
  local_hour INTEGER NOT NULL DEFAULT 9,
  local_minute INTEGER NOT NULL DEFAULT 0,
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_at INTEGER NOT NULL,
  last_run_at INTEGER,
  last_status TEXT NOT NULL DEFAULT 'pending',
  last_summary TEXT NOT NULL DEFAULT '',
  claimed_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (unixepoch()),
  UNIQUE(tool_name, resource_key)
);
CREATE INDEX IF NOT EXISTS idx_scheduled_tools_due ON scheduled_tools(enabled, next_run_at);
