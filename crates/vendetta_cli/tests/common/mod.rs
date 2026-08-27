#![allow(dead_code, unused_imports)]

use std::path::{Path, PathBuf};
use tempfile::TempDir;
use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;

pub fn create_fixture_db(dir: &TempDir) -> PathBuf {
    let db_path = dir.path().join("fixture_archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("CLI Test Group".to_string()),
        username: Some("clitest".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            date: 1700000000 + i * 60,
            sender_id: Some(peer.peer_id),
            text: Some(format!("CLI test message {i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: if i > 1 {
                Some(MessageId::new(i - 1))
            } else {
                None
            },
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: None,
            views: Some(10),
            forwards_count: Some(1),
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    db_path
}
