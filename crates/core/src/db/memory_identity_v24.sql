-- Platform-agnostic core memory identity links, aliases, and directory views.
CREATE TABLE IF NOT EXISTS memory_identity_links (
    source_pid TEXT PRIMARY KEY NOT NULL,
    target_pid TEXT NOT NULL,
    platform TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_memory_identity_target ON memory_identity_links(target_pid);

INSERT OR IGNORE INTO memory_identity_links(source_pid, target_pid, platform)
SELECT official_pid, onebot_pid, 'qq' FROM qq_memory_links;

CREATE VIEW IF NOT EXISTS memory_aliases(person_id, alias) AS
    SELECT source_pid, target_pid FROM memory_identity_links
    UNION ALL
    SELECT target_pid, source_pid FROM memory_identity_links
    UNION ALL
    SELECT a.source_pid, b.source_pid FROM memory_identity_links a
    JOIN memory_identity_links b ON a.target_pid=b.target_pid AND a.source_pid<>b.source_pid;

CREATE VIEW IF NOT EXISTS memory_directory(person_id, directory_id) AS
    SELECT p.person_id, COALESCE(target.person_id, (
        SELECT MIN(peer.person_id) FROM memory_identity_links other
        JOIN persons peer ON peer.person_id=other.source_pid
        WHERE other.target_pid=link.target_pid
    ), p.person_id)
    FROM persons p
    LEFT JOIN memory_identity_links link ON link.source_pid=p.person_id
    LEFT JOIN persons target ON target.person_id=link.target_pid;

-- Keep legacy qq_memory_links writes synchronized with core memory_identity_links.
CREATE TRIGGER IF NOT EXISTS trg_qq_memory_links_insert
AFTER INSERT ON qq_memory_links
BEGIN
    INSERT OR REPLACE INTO memory_identity_links(source_pid, target_pid, platform)
    VALUES(NEW.official_pid, NEW.onebot_pid, 'qq');
END;

CREATE TRIGGER IF NOT EXISTS trg_qq_memory_links_delete
AFTER DELETE ON qq_memory_links
BEGIN
    DELETE FROM memory_identity_links
    WHERE source_pid=OLD.official_pid AND target_pid=OLD.onebot_pid;
END;
