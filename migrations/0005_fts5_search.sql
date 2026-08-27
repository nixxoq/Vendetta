-- Migration 0005: SQLite FTS5 Full-Text Search Index
-- Creates canonical messages_fts virtual table using unicode61 tokenizer
-- Unindexed columns for fast structured filtering without bloating inverted index

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    text,
    peer_id UNINDEXED,
    message_id UNINDEXED,
    sender_id UNINDEXED,
    date UNINDEXED,
    state UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Backfill active and edited messages with non-null text
INSERT INTO messages_fts(text, peer_id, message_id, sender_id, date, state)
SELECT text, peer_id, message_id, sender_id, date, state
FROM messages
WHERE text IS NOT NULL AND state IN ('active', 'edited');

-- Trigger on INSERT
CREATE TRIGGER IF NOT EXISTS trg_messages_fts_ai AFTER INSERT ON messages
WHEN new.text IS NOT NULL AND new.state IN ('active', 'edited')
BEGIN
    INSERT INTO messages_fts(text, peer_id, message_id, sender_id, date, state)
    VALUES (new.text, new.peer_id, new.message_id, new.sender_id, new.date, new.state);
END;

-- Trigger on UPDATE
CREATE TRIGGER IF NOT EXISTS trg_messages_fts_au AFTER UPDATE ON messages
BEGIN
    DELETE FROM messages_fts WHERE peer_id = old.peer_id AND message_id = old.message_id;
    INSERT INTO messages_fts(text, peer_id, message_id, sender_id, date, state)
    SELECT new.text, new.peer_id, new.message_id, new.sender_id, new.date, new.state
    WHERE new.text IS NOT NULL AND new.state IN ('active', 'edited');
END;

-- Trigger on DELETE
CREATE TRIGGER IF NOT EXISTS trg_messages_fts_ad AFTER DELETE ON messages
BEGIN
    DELETE FROM messages_fts WHERE peer_id = old.peer_id AND message_id = old.message_id;
END;
