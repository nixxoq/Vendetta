use std::fs;
use vendetta_import::synthetic_id::*;
use vendetta_model::{FilterReason, PeerType};

#[test]
fn test_synthetic_id_determinism_and_namespace() {
    let id_1 = generate_synthetic_chat_id("Alice and Bob", PeerType::User, None);
    let id_2 = generate_synthetic_chat_id("Alice and Bob", PeerType::User, None);
    assert_eq!(id_1, id_2);

    assert!(id_1.raw() <= -8_000_000_000_000_000_000);
    assert!(id_1.raw() > -9_000_000_000_000_000_000);

    let sender_1 = generate_synthetic_sender_id(id_1, "Alice", Some("A"));
    let sender_2 = generate_synthetic_sender_id(id_1, "Alice", Some("A"));
    assert_eq!(sender_1, sender_2);

    assert!(sender_1.raw() <= -6_000_000_000_000_000_000);
    assert!(sender_1.raw() > -7_000_000_000_000_000_000);

    let sender_bob = generate_synthetic_sender_id(id_1, "Bob", Some("B"));
    assert_ne!(sender_1, sender_bob);

    let id_other = generate_synthetic_chat_id("Other Chat", PeerType::Group, None);
    assert_ne!(id_1, id_other);
}

#[test]
fn test_html_parser_joined_cluster_and_date_dividers() {
    let temp = tempfile::tempdir().unwrap();
    let export_dir = temp.path();

    let html_content = r##"<!DOCTYPE html>
<html>
<head><title>Export</title></head>
<body>
<div class="page_header"><div class="text bold">Synthetic Chat</div></div>
<div class="history">
  <div class="message service" id="message-1">
    <div class="body details">19 September 2026</div>
  </div>
  <div class="message default clearfix" id="message101">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 10:00:00 UTC+00:00">10:00</div>
      <div class="from_name">Alice</div>
      <div class="text">First message from Alice</div>
    </div>
  </div>
  <div class="message default clearfix joined" id="message102">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 10:01:00 UTC+00:00">10:01</div>
      <div class="text">Second message joined in cluster</div>
    </div>
  </div>
  <div class="message default clearfix" id="message103">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 10:02:00 UTC+00:00">10:02</div>
      <div class="from_name">Bob</div>
      <div class="reply_to details">In reply to <a href="#go_to_message101">this message</a></div>
      <div class="text">Reply from Bob to Alice</div>
    </div>
  </div>
  <div class="message service" id="message104">
    <div class="body details">Alice pinned a message</div>
  </div>
</div>
</body>
</html>"##;

    fs::write(export_dir.join("messages.html"), html_content).unwrap();

    let discovery = vendetta_import::discovery::discover_export(export_dir).unwrap();
    assert_eq!(discovery.chat_sources.len(), 1);

    let chat =
        vendetta_import::parser_html::parse_tdesktop_html(&discovery.chat_sources[0]).unwrap();
    assert_eq!(chat.name.as_deref(), Some("Synthetic Chat"));

    assert_eq!(chat.messages.len(), 4);
    assert_eq!(chat.messages[0].message_id.raw(), 101);
    assert_eq!(chat.messages[0].sender_name.as_deref(), Some("Alice"));
    assert_eq!(
        chat.messages[0].text.as_deref(),
        Some("First message from Alice")
    );

    assert_eq!(chat.messages[1].message_id.raw(), 102);
    assert_eq!(chat.messages[1].sender_name.as_deref(), Some("Alice"));
    assert_eq!(chat.messages[1].sender_id, chat.messages[0].sender_id);
    assert!(chat.messages[1].is_joined_continuation);

    assert_eq!(chat.messages[2].message_id.raw(), 103);
    assert_eq!(chat.messages[2].sender_name.as_deref(), Some("Bob"));
    assert_eq!(
        chat.messages[2].reply_to_message_id.map(|m| m.raw()),
        Some(101)
    );

    assert_eq!(chat.messages[3].message_id.raw(), 104);
    assert!(chat.messages[3].service_event.is_some());
}

#[test]
fn test_html_parser_oversized_media_placeholder() {
    let temp = tempfile::tempdir().unwrap();
    let export_dir = temp.path();

    let html_content = r##"<!DOCTYPE html>
<html>
<body>
<div class="page_header"><div class="text bold">Media Test</div></div>
<div class="history">
  <div class="message default clearfix" id="message201">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 10:00:00 UTC+00:00">10:00</div>
      <div class="from_name">User</div>
      <div class="media clearfix pull_left media_file">
        <div class="body">
          <div class="title bold">large_file.zip</div>
          <div class="status details">Not included, exceeds maximum size</div>
        </div>
      </div>
    </div>
  </div>
</div>
</body>
</html>"##;

    fs::write(export_dir.join("messages.html"), html_content).unwrap();
    let discovery = vendetta_import::discovery::discover_export(export_dir).unwrap();
    let chat =
        vendetta_import::parser_html::parse_tdesktop_html(&discovery.chat_sources[0]).unwrap();

    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages[0].media.len(), 1);
    assert!(chat.messages[0].media[0].is_skipped);
    assert_eq!(
        chat.messages[0].media[0].skip_reason,
        Some(FilterReason::SizeAboveMax)
    );
}

#[test]
fn test_json_parser_single_and_full_account() {
    let temp = tempfile::tempdir().unwrap();
    let export_dir = temp.path();

    let json_content = r##"{
      "name": "JSON Single Chat",
      "type": "personal_chat",
      "id": 12345,
      "messages": [
        {
          "id": 1,
          "type": "message",
          "date_unixtime": "1700000000",
          "from": "Alice",
          "from_id": "user111",
          "text": "Hello world",
          "text_entities": [
            { "type": "bold", "text": "Hello" },
            { "type": "plain", "text": " world" }
          ],
          "reactions": [
            { "type": "emoji", "count": 2, "emoji": "👍" }
          ]
        }
      ]
    }"##;

    fs::write(export_dir.join("result.json"), json_content).unwrap();
    let discovery = vendetta_import::discovery::discover_export(export_dir).unwrap();
    let chats =
        vendetta_import::parser_json::parse_tdesktop_json(&discovery.chat_sources[0]).unwrap();

    assert_eq!(chats.len(), 1);
    let chat = &chats[0];
    assert_eq!(chat.peer_id.raw(), 12345);
    assert_eq!(chat.messages.len(), 1);

    let msg = &chat.messages[0];
    assert_eq!(msg.message_id.raw(), 1);
    assert_eq!(msg.sender_id.map(|p| p.raw()), Some(111));
    assert_eq!(msg.text.as_deref(), Some("Hello world"));
    assert_eq!(msg.entities.len(), 1);
    assert_eq!(msg.entities[0].offset_utf16, 0);
    assert_eq!(msg.entities[0].length_utf16, 5); // "Hello"
    assert_eq!(msg.reactions.len(), 1);
    assert_eq!(msg.reactions[0].emoji, "👍");
    assert_eq!(msg.reactions[0].count, 2);
}
