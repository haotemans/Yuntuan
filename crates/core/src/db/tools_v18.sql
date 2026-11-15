CREATE TABLE IF NOT EXISTS tool_contracts (
 name TEXT PRIMARY KEY,
 definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tool_invocation_records (
 invocation_id TEXT PRIMARY KEY REFERENCES task_invocations(id),
 task_id TEXT NOT NULL REFERENCES tasks(task_id),
 revision INTEGER NOT NULL,
 call_id TEXT NOT NULL,
 protocol TEXT NOT NULL CHECK(protocol IN ('json','native')),
 definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 status TEXT NOT NULL,
 error_json TEXT,
 elapsed_ms INTEGER NOT NULL DEFAULT 0,
 UNIQUE(task_id,revision,call_id)
);
