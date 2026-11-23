CREATE TABLE IF NOT EXISTS qq_memory_links (
    official_pid TEXT PRIMARY KEY NOT NULL,
    onebot_pid TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_qq_memory_onebot ON qq_memory_links(onebot_pid);

CREATE VIEW IF NOT EXISTS qq_memory_aliases(person_id, alias) AS
    SELECT official_pid, onebot_pid FROM qq_memory_links
    UNION ALL
    SELECT onebot_pid, official_pid FROM qq_memory_links
    UNION ALL
    SELECT a.official_pid, b.official_pid FROM qq_memory_links a
    JOIN qq_memory_links b ON a.onebot_pid=b.onebot_pid AND a.official_pid<>b.official_pid;
