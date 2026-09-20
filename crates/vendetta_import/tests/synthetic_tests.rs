use std::{
    fs::{self, File},
    io::Write,
};

use vendetta_import::{
    ConvertOptions, ImportOptions, convert_tdesktop, import_tdesktop, synthetic_id::*,
};
use vendetta_model::{FilterReason, PeerType};

#[test]
fn test_synthetic_id_determinism_and_namespace() {
    let id_1 = generate_synthetic_chat_id("Alice and Bob", PeerType::User, None);
    let id_2 = generate_synthetic_chat_id("Alice and Bob", PeerType::User, None);
    assert_eq!(id_1, id_2);

    // Reserved chat namespace: [-9_000_000_000_000_000_000, -8_000_000_000_000_000_000)
    assert!(id_1.raw() <= -8_000_000_000_000_000_000);
    assert!(id_1.raw() > -9_000_000_000_000_000_000);

    let sender_1 = generate_synthetic_sender_id(id_1, "Alice", Some("A"));
    let sender_2 = generate_synthetic_sender_id(id_1, "Alice", Some("A"));
    assert_eq!(sender_1, sender_2);

    // Reserved sender namespace: [-7_000_000_000_000_000_000, -6_000_000_000_000_000_000)
    assert!(sender_1.raw() <= -6_000_000_000_000_000_000);
    assert!(sender_1.raw() > -7_000_000_000_000_000_000);

    // Distinct identities produce distinct IDs
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

    // Date divider id=-1 must be filtered out; messages: 101, 102, 103, 104
    assert_eq!(chat.messages.len(), 4);

    // Msg 101
    assert_eq!(chat.messages[0].message_id.raw(), 101);
    assert_eq!(chat.messages[0].sender_name.as_deref(), Some("Alice"));
    assert_eq!(
        chat.messages[0].text.as_deref(),
        Some("First message from Alice")
    );

    // Msg 102: joined message inherits sender "Alice"
    assert_eq!(chat.messages[1].message_id.raw(), 102);
    assert_eq!(chat.messages[1].sender_name.as_deref(), Some("Alice"));
    assert_eq!(chat.messages[1].sender_id, chat.messages[0].sender_id);
    assert!(chat.messages[1].is_joined_continuation);

    // Msg 103: Bob with reply to 101
    assert_eq!(chat.messages[2].message_id.raw(), 103);
    assert_eq!(chat.messages[2].sender_name.as_deref(), Some("Bob"));
    assert_eq!(
        chat.messages[2].reply_to_message_id.map(|m| m.raw()),
        Some(101)
    );

    // Msg 104: Service message
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

#[test]
fn test_zip_path_traversal_protection() {
    let temp = tempfile::tempdir().unwrap();
    let bad_zip_path = temp.path().join("malicious.zip");

    {
        let file = File::create(&bad_zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();

        // Attempt Zip Slip using relative path escaping
        zip.start_file("../outside.txt", options).unwrap();
        zip.write_all(b"pwned").unwrap();
        zip.finish().unwrap();
    }

    let res = vendetta_import::discovery::extract_zip_safely(&bad_zip_path);
    assert!(res.is_err());
    let err_msg = res.err().unwrap().to_string();
    assert!(err_msg.contains("Path traversal attempt") || err_msg.contains("Illegal path"));
}

#[test]
fn test_import_and_convert_lifecycle_and_guards() {
    let temp = tempfile::tempdir().unwrap();
    let export_dir = temp.path().join("export");
    fs::create_dir_all(&export_dir).unwrap();

    let html_content = r##"<!DOCTYPE html>
<html>
<body>
<div class="page_header"><div class="text bold">Lifecycle Chat</div></div>
<div class="history">
  <div class="message default clearfix" id="message1">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 12:00:00 UTC+00:00">12:00</div>
      <div class="from_name">Author</div>
      <div class="text">Testing lifecycle</div>
    </div>
  </div>
</div>
</body>
</html>"##;
    fs::write(export_dir.join("messages.html"), html_content).unwrap();

    let db_path = temp.path().join("archive.db");

    // 1. import_tdesktop into fresh destination succeeds
    let import_opts = ImportOptions {
        source_path: export_dir.clone(),
        archive_path: db_path.clone(),
        media_dir: None,
    };
    let import_summary = import_tdesktop(&import_opts).unwrap();
    assert_eq!(import_summary.messages_count, 1);
    assert!(db_path.is_file());

    // 2. import_tdesktop into existing database fails (Fresh Guard)
    let reimport_err = import_tdesktop(&import_opts);
    assert!(reimport_err.is_err());
    assert!(
        reimport_err
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );

    // 3. convert_tdesktop into new destination succeeds
    let html_out = temp.path().join("html_out");
    let convert_opts = ConvertOptions {
        source_path: export_dir.clone(),
        output_dir: html_out.clone(),
        replace: false,
        ..Default::default()
    };
    let conv_summary = convert_tdesktop(&convert_opts).unwrap();
    assert_eq!(conv_summary.messages_count, 1);
    assert!(html_out.join("manifest.json").is_file());

    // 4. convert_tdesktop into existing destination without --replace fails
    let re_conv_err = convert_tdesktop(&convert_opts);
    assert!(re_conv_err.is_err());
    assert!(
        re_conv_err
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );

    // 5. convert_tdesktop into existing destination with --replace succeeds
    let convert_replace_opts = ConvertOptions {
        source_path: export_dir,
        output_dir: html_out.clone(),
        replace: true,
        ..Default::default()
    };
    let conv_replace_summary = convert_tdesktop(&convert_replace_opts).unwrap();
    assert_eq!(conv_replace_summary.messages_count, 1);
}

#[test]
fn test_full_pipeline_direction_and_service_card() {
    let temp = tempfile::tempdir().unwrap();
    let export_dir = temp.path().join("source_export");
    fs::create_dir_all(export_dir.join("photos")).unwrap();

    // Create a dummy photo file
    fs::write(
        export_dir.join("photos/profile_photo.jpg"),
        b"fake jpeg content",
    )
    .unwrap();

    let html_content = r##"<!DOCTYPE html>
<html>
<head><title>Export</title></head>
<body>
<div class="page_header"><div class="text bold">Danilo</div></div>
<div class="history">
  <div class="message default clearfix" id="message1">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 12:00:00 UTC+00:00">12:00</div>
      <div class="from_name">Danilo</div>
      <div class="text">Hello Tolya</div>
    </div>
  </div>
  <div class="message default clearfix" id="message2">
    <div class="body">
      <div class="pull_right date details" title="19.09.2026 12:01:00 UTC+00:00">12:01</div>
      <div class="from_name">Tolya Stepanov</div>
      <div class="text">Hi Danilo</div>
    </div>
  </div>
  <div class="message service" id="message3">
    <div class="body details">
      Tolya Stepanov suggests to use this photo
    </div>
    <div class="userpic_wrap">
      <a class="userpic_link" href="photos/profile_photo.jpg">
        <div class="userpic userpic_photo" style="background-image: url('photos/profile_photo.jpg');"></div>
      </a>
    </div>
  </div>
</div>
</body>
</html>"##;
    fs::write(export_dir.join("messages.html"), html_content).unwrap();

    let db_path = temp.path().join("archive.db");
    let media_dir = temp.path().join("media");

    let import_opts = ImportOptions {
        source_path: export_dir.clone(),
        archive_path: db_path.clone(),
        media_dir: Some(media_dir.clone()),
    };
    let summary = import_tdesktop(&import_opts).unwrap();
    assert_eq!(summary.messages_count, 3);

    // 1. Verify ArchiveDb contents and raw_tl `out` direction
    let db = vendetta_storage::ArchiveDb::open(&db_path).unwrap();
    let peers = db.list_peers().unwrap();
    let chat_peer = peers
        .iter()
        .find(|p| p.name.as_deref() == Some("Danilo"))
        .expect("Chat peer must exist");
    let chat_id = chat_peer.peer_id;

    let msg1 = db
        .get_message(vendetta_model::MessageKey::new(
            chat_id,
            vendetta_model::MessageId::new(1),
        ))
        .unwrap()
        .unwrap();
    let msg2 = db
        .get_message(vendetta_model::MessageKey::new(
            chat_id,
            vendetta_model::MessageId::new(2),
        ))
        .unwrap()
        .unwrap();
    let msg3 = db
        .get_message(vendetta_model::MessageKey::new(
            chat_id,
            vendetta_model::MessageId::new(3),
        ))
        .unwrap()
        .unwrap();

    use grammers_tl_types::{self as tl, Deserializable};
    // Message 1 (Danilo, incoming): out = false
    match tl::enums::Message::from_bytes(msg1.raw_tl.as_ref().unwrap()).unwrap() {
        tl::enums::Message::Message(m) => {
            assert!(!m.out, "Message 1 must be incoming (out = false)")
        }
        _ => panic!("Expected regular message for msg1"),
    }
    // Message 2 (Tolya, outgoing): out = true
    match tl::enums::Message::from_bytes(msg2.raw_tl.as_ref().unwrap()).unwrap() {
        tl::enums::Message::Message(m) => {
            assert!(m.out, "Message 2 must be outgoing (out = true)")
        }
        _ => panic!("Expected regular message for msg2"),
    }
    // Message 3 (Service, suggested photo by Tolya): out = true
    match tl::enums::Message::from_bytes(msg3.raw_tl.as_ref().unwrap()).unwrap() {
        tl::enums::Message::Service(s) => {
            assert!(s.out, "Service msg 3 must be outgoing (out = true)")
        }
        _ => panic!("Expected service message for msg3"),
    }

    // Verify service message media is persisted in ArchiveDb
    let msg3_media = db
        .get_message_media_with_roles(chat_id, vendetta_model::MessageId::new(3))
        .unwrap();
    assert_eq!(
        msg3_media.len(),
        1,
        "Service message 3 must have 1 attached media item"
    );
    assert_eq!(msg3_media[0].0.kind, vendetta_model::MediaKind::Photo);

    // 2. Completely delete the source export directory to prove independence
    fs::remove_dir_all(&export_dir).unwrap();
    assert!(!export_dir.exists());

    // 3. Export HTML directly from the self-contained native ArchiveDb
    let html_out = temp.path().join("html_export");
    let options = vendetta_render::ExportOptions {
        output_dir: html_out.clone(),
        presentation_mode: vendetta_render::PresentationMode::TelegramLike,
        media_mode: vendetta_render::MediaMode::Copy,
        theme: vendetta_render::ThemeMode::System,
        chunk_size: 250,
        replace: true,
        media_src_dir: Some(media_dir),
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: false,
        build_date_index: false,
        target_peers: None,
    };
    let exporter = vendetta_render::HtmlArchiveExporter::new(&db, options);
    let render_summary = exporter.export().unwrap();
    assert_eq!(render_summary.dialogs_count, 1);

    // Read generated chat page
    let rel_chunk_path =
        vendetta_render::url_builder::ArchiveUrlBuilder::chunk_file_rel(chat_id, 0);
    let page_path = html_out.join(rel_chunk_path);
    let html = fs::read_to_string(&page_path).unwrap();

    // Verify incoming and outgoing message bubbles
    assert!(
        html.contains("msg-incoming") || html.contains("message-bubble"),
        "Contains incoming styling"
    );
    assert!(
        html.contains("msg-outgoing"),
        "Message 2 must have msg-outgoing CSS class"
    );
    assert!(html.contains("Hi Danilo"), "Must contain Message 2 text");

    // Verify rich service card markup
    assert!(
        html.contains("service-card"),
        "Must contain service-card container"
    );
    assert!(
        html.contains("service-card-avatar"),
        "Must contain service-card avatar preview"
    );
    assert!(
        html.contains("You suggested this photo for Danilo") && html.contains("Telegram profile."),
        "Must contain adapted text"
    );
    assert!(
        html.contains("service-card-btn media-lightbox-trigger"),
        "Must contain lightbox button"
    );
    assert!(
        html.contains("View Photo"),
        "Button text must be View Photo"
    );
}
