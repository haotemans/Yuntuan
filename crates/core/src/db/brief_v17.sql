-- Task requirements survive history compaction. No existing facts or tasks are rewritten.
CREATE TABLE IF NOT EXISTS task_brief_versions (
 task_id TEXT NOT NULL REFERENCES tasks(task_id),
 revision INTEGER NOT NULL,
 body TEXT NOT NULL CHECK(json_valid(body)),
 source_msg_id INTEGER REFERENCES messages(msg_id),
 event_kind TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 PRIMARY KEY(task_id,revision)
);
CREATE UNIQUE INDEX IF NOT EXISTS task_brief_source_once
 ON task_brief_versions(task_id,source_msg_id,event_kind) WHERE source_msg_id IS NOT NULL;
