CREATE TABLE IF NOT EXISTS platform_messages (
    msg_id INTEGER PRIMARY KEY REFERENCES messages(msg_id) ON DELETE CASCADE,
    platform TEXT NOT NULL,
    account TEXT NOT NULL,
    message_key TEXT NOT NULL,
    recipient TEXT NOT NULL,
    next_seq INTEGER NOT NULL DEFAULT 1 CHECK(next_seq >= 1),
    UNIQUE(platform, account, message_key)
);
