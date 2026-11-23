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

-- A directory card has a real person ID, even before the OneBot identity is seen.
CREATE VIEW IF NOT EXISTS qq_memory_directory(person_id, directory_id) AS
    SELECT p.person_id, COALESCE(onebot.person_id, (
        SELECT MIN(peer.person_id) FROM qq_memory_links other
        JOIN persons peer ON peer.person_id=other.official_pid
        WHERE other.onebot_pid=link.onebot_pid
    ), p.person_id)
    FROM persons p
    LEFT JOIN qq_memory_links link ON link.official_pid=p.person_id
    LEFT JOIN persons onebot ON onebot.person_id=link.onebot_pid;
