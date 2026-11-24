-- Agent schedules use the existing task runner, not a second execution engine.
CREATE TABLE IF NOT EXISTS agent_schedules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  goal TEXT NOT NULL,
  actor_pid TEXT NOT NULL,
  allowed_tools TEXT NOT NULL,
  max_calls INTEGER NOT NULL,
  timeout_secs INTEGER NOT NULL,
  local_hour INTEGER NOT NULL,
  local_minute INTEGER NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_agent_schedules_due ON agent_schedules(enabled,next_run_at);
CREATE TABLE IF NOT EXISTS agent_schedule_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  schedule_id INTEGER REFERENCES agent_schedules(id) ON DELETE SET NULL,
  task_id TEXT UNIQUE REFERENCES tasks(task_id),
  triggered_at INTEGER NOT NULL,
  error TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_schedule_runs ON agent_schedule_runs(schedule_id,id DESC);
