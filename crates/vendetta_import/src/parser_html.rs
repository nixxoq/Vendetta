use std::{fs, path::Path};

use scraper::{ElementRef, Html, Node, Selector};
use tracing::debug;
use vendetta_core::ymd_to_days;
use vendetta_model::{FilterReason, MediaKind, MediaRole, MessageId, PeerType};

use crate::{
    discovery::TDesktopChatSource,
    error::{ImportError, ImportResult},
    model::{
        ImportChat, ImportEntityKind, ImportForwardInfo, ImportMediaItem, ImportMessage,
        ImportServiceEvent, ImportTextEntity,
    },
    synthetic_id::{generate_synthetic_chat_id, generate_synthetic_sender_id},
};

pub fn parse_tdesktop_html(source: &TDesktopChatSource) -> ImportResult<ImportChat> {
    if source.entry_files.is_empty() {
        return Err(ImportError::ParsingFailed(format!(
            "No HTML chunk files found in {}",
            source.base_dir.display()
        )));
    }

    let mut chat_title: Option<String> = source.title_hint.clone();
    let mut messages: Vec<ImportMessage> = Vec::new();

    let mut current_cluster_sender_name: Option<String> = None;
    let mut current_cluster_userpic: Option<String> = None;
    let mut last_seen_date: Option<i64> = None;

    let page_header_sel = Selector::parse(".page_header .text.bold")
        .map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let message_sel =
        Selector::parse(".message").map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let date_sel = Selector::parse(".pull_right.date.details")
        .map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let from_name_sel =
        Selector::parse(".from_name").map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let userpic_sel =
        Selector::parse(".userpic").map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let reply_to_sel = Selector::parse(".reply_to.details a")
        .map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let text_sel =
        Selector::parse(".text").map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;
    let forwarded_sel = Selector::parse(".forwarded.body .from_name")
        .map_err(|e| ImportError::ParsingFailed(format!("{e:?}")))?;

    if chat_title.is_none()
        && let Some(first_file) = source.entry_files.first()
    {
        let content = fs::read_to_string(first_file)?;
        let doc = Html::parse_document(&content);
        if let Some(el) = doc.select(&page_header_sel).next() {
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                chat_title = Some(text);
            }
        }
    }

    let final_title = chat_title.unwrap_or_else(|| {
        source
            .base_dir
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("Chat")
            .to_string()
    });

    let detected_type = PeerType::User;
    let chat_id = generate_synthetic_chat_id(
        &final_title,
        detected_type,
        source.internal_discriminator.as_deref(),
    );

    for file_path in &source.entry_files {
        let content = fs::read_to_string(file_path)?;
        let doc = Html::parse_document(&content);

        for msg_el in doc.select(&message_sel) {
            let elem_val = msg_el.value();
            let raw_id = elem_val
                .attr("id")
                .and_then(|id| id.strip_prefix("message"))
                .and_then(|id| id.parse::<i64>().ok())
                .unwrap_or(0);

            // Filter out date dividers and non-positive IDs (e.g. id="message-1")
            if raw_id <= 0 {
                let divider_text = msg_el.text().collect::<String>();
                if let Some(ts) = parse_tdesktop_date_divider(&divider_text) {
                    last_seen_date = Some(ts);
                }
                continue;
            }

            let message_id = MessageId::new(raw_id);
            let is_service = elem_val.has_class("service", scraper::CaseSensitivity::CaseSensitive);
            let is_joined = elem_val.has_class("joined", scraper::CaseSensitivity::CaseSensitive);

            let parsed_date = msg_el
                .select(&date_sel)
                .next()
                .and_then(|el| el.value().attr("title"))
                .and_then(parse_tdesktop_date);

            if let Some(ts) = parsed_date {
                last_seen_date = Some(ts);
            }

            let date = parsed_date
                .or(last_seen_date)
                .unwrap_or_else(vendetta_core::now_unix_secs);

            // Handle service messages
            if is_service {
                static SERVICE_DETAILS_SEL: std::sync::LazyLock<Selector> =
                    std::sync::LazyLock::new(|| {
                        Selector::parse("div.body.details, div.details").expect("static selector")
                    });
                let service_text = msg_el
                    .select(&SERVICE_DETAILS_SEL)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| msg_el.text().collect::<String>().trim().to_string());

                let media = extract_media_items(msg_el, &source.base_dir);

                messages.push(ImportMessage {
                    message_id,
                    date,
                    sender_id: None,
                    sender_name: None,
                    text: Some(service_text.clone()),
                    entities: Vec::new(),
                    edit_date: None,
                    reply_to_message_id: None,
                    forward_info: None,
                    reactions: Vec::new(),
                    media,
                    service_event: Some(ImportServiceEvent::ActionText(service_text)),
                    is_joined_continuation: false,
                    is_outgoing: false,
                });
                continue;
            }

            if !is_joined {
                if let Some(fn_el) = msg_el.select(&from_name_sel).next() {
                    let name = fn_el.text().collect::<String>().trim().to_string();
                    if !name.is_empty() {
                        current_cluster_sender_name = Some(name);
                    }
                }
                if let Some(up_el) = msg_el.select(&userpic_sel).next() {
                    let userpic_class = up_el
                        .value()
                        .classes()
                        .find(|c| c.starts_with("userpic"))
                        .map(ToString::to_string);
                    current_cluster_userpic = userpic_class.or_else(|| {
                        let initials = up_el.text().collect::<String>().trim().to_string();
                        if !initials.is_empty() {
                            Some(initials)
                        } else {
                            None
                        }
                    });
                } else {
                    current_cluster_userpic = None;
                }
            }

            let sender_name = current_cluster_sender_name.clone();
            let userpic_marker = current_cluster_userpic.clone();

            let reply_to_message_id = msg_el
                .select(&reply_to_sel)
                .next()
                .and_then(|el| el.value().attr("href"))
                .and_then(extract_reply_id);

            let forward_info = msg_el.select(&forwarded_sel).next().map(|el| {
                let name = el.text().collect::<String>().trim().to_string();
                ImportForwardInfo {
                    from_name: Some(name),
                    date: None,
                }
            });

            // Parse text and entities simultaneously to strictly preserve Invariant 3.4
            let (text, entities) = if let Some(t_el) = msg_el.select(&text_sel).next() {
                let mut emitted_text = String::new();
                let mut current_utf16_len = 0usize;
                let mut entities = Vec::new();

                traverse_text_element(
                    t_el,
                    &mut emitted_text,
                    &mut current_utf16_len,
                    &mut entities,
                );

                let trimmed_end = emitted_text.trim_end_matches(['\r', '\n', ' ']);
                let trimmed_end_len_u16 = trimmed_end.encode_utf16().count();
                if trimmed_end_len_u16 < current_utf16_len {
                    emitted_text.truncate(trimmed_end.len());
                    for ent in &mut entities {
                        if ent.offset_utf16 >= trimmed_end_len_u16 {
                            ent.length_utf16 = 0;
                        } else if ent.offset_utf16 + ent.length_utf16 > trimmed_end_len_u16 {
                            ent.length_utf16 = trimmed_end_len_u16 - ent.offset_utf16;
                        }
                    }
                    entities.retain(|e| e.length_utf16 > 0);
                }

                let text_opt = if emitted_text.is_empty() {
                    None
                } else {
                    Some(emitted_text)
                };
                (text_opt, entities)
            } else {
                (None, Vec::new())
            };

            // Parse media wrappers
            let media = extract_media_items(msg_el, &source.base_dir);

            let sender_id = sender_name
                .as_ref()
                .map(|name| generate_synthetic_sender_id(chat_id, name, userpic_marker.as_deref()));

            messages.push(ImportMessage {
                message_id,
                date,
                sender_id,
                sender_name,
                text,
                entities,
                edit_date: None,
                reply_to_message_id,
                forward_info,
                reactions: Vec::new(),
                media,
                service_event: None,
                is_joined_continuation: is_joined,
                is_outgoing: false,
            });
        }
    }

    debug!(
        "Parsed HTML chat '{}' ({}) with {} messages",
        final_title,
        chat_id,
        messages.len()
    );

    Ok(ImportChat {
        peer_id: chat_id,
        peer_type: detected_type,
        name: Some(final_title),
        username: None,
        messages,
    })
}

fn traverse_text_element(
    el: ElementRef<'_>,
    emitted: &mut String,
    utf16_len: &mut usize,
    entities: &mut Vec<ImportTextEntity>,
) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                let decoded = decode_html_entities(&t.text);
                let slice = if emitted.is_empty() {
                    decoded.trim_start_matches(['\r', '\n'])
                } else {
                    &decoded
                };
                if !slice.is_empty() {
                    let u16_count = slice.encode_utf16().count();
                    emitted.push_str(slice);
                    *utf16_len += u16_count;
                }
            }
            Node::Element(elem) => {
                let tag = elem.name();
                if tag == "br" {
                    if !emitted.is_empty() {
                        emitted.push('\n');
                        *utf16_len += 1;
                    }
                    continue;
                }

                let entity_kind = match tag {
                    "b" | "strong" => Some(ImportEntityKind::Bold),
                    "i" | "em" => Some(ImportEntityKind::Italic),
                    "u" => Some(ImportEntityKind::Underline),
                    "s" | "strike" => Some(ImportEntityKind::Strike),
                    "code" => Some(ImportEntityKind::Code),
                    "pre" => {
                        let lang = elem.attr("class").and_then(|c| c.strip_prefix("language-"));
                        Some(ImportEntityKind::Pre(lang.map(ToString::to_string)))
                    }
                    "a" => {
                        let href = elem.attr("href").unwrap_or("").to_string();
                        Some(ImportEntityKind::TextUrl(href))
                    }
                    "span" if elem.classes().any(|c| c == "spoiler") => {
                        Some(ImportEntityKind::Spoiler)
                    }
                    "blockquote" => Some(ImportEntityKind::Blockquote),
                    _ => None,
                };

                let child_ref = ElementRef::wrap(child);
                if let Some(c_ref) = child_ref {
                    let start_u16 = *utf16_len;
                    traverse_text_element(c_ref, emitted, utf16_len, entities);
                    let end_u16 = *utf16_len;

                    if let Some(kind) = entity_kind {
                        let len_u16 = end_u16 - start_u16;
                        if len_u16 > 0 {
                            entities.push(ImportTextEntity {
                                kind,
                                offset_utf16: start_u16,
                                length_utf16: len_u16,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '&' {
            let mut entity = String::new();
            let mut found_semicolon = false;
            for next_c in chars.by_ref() {
                if next_c == ';' {
                    found_semicolon = true;
                    break;
                }
                entity.push(next_c);
                if entity.len() > 10 {
                    break;
                }
            }
            if found_semicolon {
                match entity.as_str() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" | "#39" => out.push('\''),
                    "nbsp" => out.push('\u{00A0}'),
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                        out.push(';');
                    }
                }
            } else {
                out.push('&');
                out.push_str(&entity);
            }
        } else {
            out.push(c);
        }
    }

    out
}

pub fn parse_tdesktop_date(title: &str) -> Option<i64> {
    let mut parts = title.split_whitespace();
    let date_str = parts.next()?;
    let time_str = parts.next()?;
    let tz_str = parts.next();

    let mut date_parts = date_str.split('.');
    let d: u32 = date_parts.next()?.parse().ok()?;
    let m: u32 = date_parts.next()?.parse().ok()?;
    let y: i32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }
    let days = ymd_to_days(y, m, d);

    let mut time_parts = time_str.split(':');
    let h: i64 = time_parts.next()?.parse().ok()?;
    let min: i64 = time_parts.next()?.parse().ok()?;
    let sec: i64 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some() {
        return None;
    }

    let local_secs = days * 86400 + h * 3600 + min * 60 + sec;

    let offset_secs = match tz_str {
        Some(tz) => {
            if let Some(sign_pos) = tz.find('+') {
                let (oh, om) = tz[sign_pos + 1..].split_once(':')?;
                let oh: i64 = oh.parse().ok()?;
                let om: i64 = om.parse().ok()?;
                oh * 3600 + om * 60
            } else if let Some(sign_neg) = tz.find('-') {
                let (oh, om) = tz[sign_neg + 1..].split_once(':')?;
                let oh: i64 = oh.parse().ok()?;
                let om: i64 = om.parse().ok()?;
                -(oh * 3600 + om * 60)
            } else {
                0
            }
        }
        None => 0,
    };

    Some(local_secs - offset_secs)
}

pub fn parse_tdesktop_date_divider(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    if let Some((d, m, y)) = parse_numeric_date(text)
        && (1..=31).contains(&d)
        && (1..=12).contains(&m)
        && y >= 1970
    {
        let days = ymd_to_days(y, m, d);
        return Some(days * 86400);
    }

    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| {
            let lower = w.to_lowercase();
            !matches!(
                lower.as_str(),
                "de" | "г." | "г" | "р." | "р" | "года" | "року" | "d'"
            )
        })
        .collect();

    if words.len() >= 3 {
        let day_str = words[0].trim_end_matches('.');
        let year_str = words.last()?.trim_end_matches('.');

        let d: u32 = day_str.parse().ok()?;
        let y: i32 = year_str.parse().ok()?;
        if !(1..=31).contains(&d) || y < 1970 {
            return None;
        }

        let month_str = words[1].to_lowercase();
        let m = match_month_name(&month_str)?;

        let days = ymd_to_days(y, m, d);
        return Some(days * 86400);
    }

    None
}

fn parse_numeric_date(text: &str) -> Option<(u32, u32, i32)> {
    if text.len() >= 8 {
        let parts: Vec<&str> = text.split(['-', '.', '/']).collect();
        if parts.len() == 3 {
            if parts[0].len() == 4 {
                let y: i32 = parts[0].parse().ok()?;
                let m: u32 = parts[1].parse().ok()?;
                let d: u32 = parts[2].parse().ok()?;
                return Some((d, m, y));
            } else if parts[2].len() == 4 {
                let d: u32 = parts[0].parse().ok()?;
                let m: u32 = parts[1].parse().ok()?;
                let y: i32 = parts[2].parse().ok()?;
                return Some((d, m, y));
            }
        }
    }
    None
}

fn match_month_name(month: &str) -> Option<u32> {
    let m = month.trim_end_matches('.').to_lowercase();
    match m.as_str() {
        "january" | "jan" | "января" | "январь" | "янв" | "січня" | "січень" | "січ" | "januar"
        | "janvier" | "enero" | "gennaio" => Some(1),

        "february" | "feb" | "февраля" | "февраль" | "фев" | "лютого" | "лютий" | "лют"
        | "februar" | "février" | "fevrier" | "febrero" | "febbraio" => Some(2),

        "march" | "mar" | "марта" | "март" | "мар" | "березня" | "березень" | "бер" | "märz"
        | "maerz" | "mars" | "marzo" => Some(3),

        "april" | "apr" | "апреля" | "апрель" | "апр" | "квітня" | "квітень" | "квіт" | "avril"
        | "abril" | "aprile" => Some(4),

        "may" | "мая" | "май" | "травня" | "травень" | "трав" | "mai" | "mayo" | "maggio" => {
            Some(5)
        }

        "june" | "jun" | "июня" | "июнь" | "июн" | "червня" | "червень" | "черв" | "juni"
        | "juin" | "junio" | "giugno" => Some(6),

        "july" | "jul" | "июля" | "июль" | "июл" | "липня" | "липень" | "лип" | "juli"
        | "juillet" | "julio" | "luglio" => Some(7),

        "august" | "aug" | "августа" | "август" | "авг" | "серпня" | "серпень" | "серп"
        | "août" | "aout" | "agosto" => Some(8),

        "september" | "sep" | "sept" | "сентября" | "сентябрь" | "сен" | "сент" | "вересня"
        | "вересень" | "вер" | "septembre" | "septiembre" | "settembre" => Some(9),

        "october" | "oct" | "октября" | "октябрь" | "окт" | "жовтня" | "жовтень" | "жовт"
        | "oktober" | "octobre" | "octubre" | "ottobre" => Some(10),

        "november" | "nov" | "ноября" | "ноябрь" | "ноя" | "нояб" | "листопада" | "листопад"
        | "лист" | "novembre" | "noviembre" => Some(11),

        "december" | "dec" | "декабря" | "декабрь" | "дек" | "грудня" | "грудень" | "груд"
        | "dezember" | "décembre" | "decembre" | "diciembre" | "dicembre" => Some(12),

        _ => None,
    }
}

fn extract_reply_id(href: &str) -> Option<MessageId> {
    let pos = href.rfind("message")?;
    let after = &href[pos + "message".len()..];
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    let digits = &after[..end];
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok().map(MessageId::new)
    }
}

static PHOTO_SEL: std::sync::LazyLock<Selector> = std::sync::LazyLock::new(|| {
    Selector::parse("a.photo_wrap, a.media_photo").expect("static selector")
});
static VIDEO_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.video_file_wrap").expect("static selector"));
static ANIMATED_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.animated_wrap").expect("static selector"));
static STICKER_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.sticker_wrap").expect("static selector"));
static VOICE_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.media_voice_message").expect("static selector"));
static AUDIO_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.media_audio_file").expect("static selector"));
static ROUND_SEL: std::sync::LazyLock<Selector> =
    std::sync::LazyLock::new(|| Selector::parse("a.media_video").expect("static selector"));
static FILE_SEL: std::sync::LazyLock<Selector> = std::sync::LazyLock::new(|| {
    Selector::parse("a.media_file, div.media_file a").expect("static selector")
});
static USERPIC_LINK_SEL: std::sync::LazyLock<Selector> = std::sync::LazyLock::new(|| {
    Selector::parse("div.userpic_wrap a.userpic_link, a.userpic_link").expect("static selector")
});

fn extract_media_items(msg_el: ElementRef<'_>, base_dir: &Path) -> Vec<ImportMediaItem> {
    let mut items = Vec::new();

    // Check for size-limited placeholder:
    // <div class="media clearfix pull_left media_file"><div class="status details">Not included, exceeds maximum size</div></div>
    let text = msg_el.text().collect::<String>();
    if text.contains("exceeds maximum size") || text.contains("Not included") {
        items.push(ImportMediaItem {
            source_path: None,
            file_name: None,
            kind: MediaKind::Document,
            mime_type: None,
            size_bytes: None,
            is_skipped: true,
            skip_reason: Some(FilterReason::SizeAboveMax),
            role: MediaRole::Attachment,
        });
        return items;
    }

    let media_specs = [
        (
            &*PHOTO_SEL,
            MediaKind::Photo,
            Some("image/jpeg"),
            MediaRole::Attachment,
        ),
        (
            &*USERPIC_LINK_SEL,
            MediaKind::Photo,
            Some("image/jpeg"),
            MediaRole::Attachment,
        ),
        (
            &*ANIMATED_SEL,
            MediaKind::Animation,
            Some("video/mp4"),
            MediaRole::Attachment,
        ),
        (
            &*STICKER_SEL,
            MediaKind::Sticker,
            Some("image/webp"),
            MediaRole::Sticker,
        ),
        (
            &*VIDEO_SEL,
            MediaKind::Video,
            Some("video/mp4"),
            MediaRole::Attachment,
        ),
        (
            &*VOICE_SEL,
            MediaKind::Voice,
            Some("audio/ogg"),
            MediaRole::Voice,
        ),
        (
            &*AUDIO_SEL,
            MediaKind::Audio,
            Some("audio/mp3"),
            MediaRole::Attachment,
        ),
        (
            &*ROUND_SEL,
            MediaKind::VideoNote,
            Some("video/mp4"),
            MediaRole::VideoNote,
        ),
        (&*FILE_SEL, MediaKind::Document, None, MediaRole::Attachment),
    ];

    let mut seen_hrefs = std::collections::HashSet::new();

    for (sel, kind, mime, role) in media_specs {
        for el in msg_el.select(sel) {
            if let Some(href) = el.value().attr("href") {
                if !seen_hrefs.insert(href.to_string()) {
                    continue;
                }
                let path = base_dir.join(href);
                let name = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .map(ToString::to_string);
                items.push(ImportMediaItem {
                    source_path: Some(path),
                    file_name: name,
                    kind,
                    mime_type: mime.map(ToString::to_string),
                    size_bytes: None,
                    is_skipped: false,
                    skip_reason: None,
                    role,
                });
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tdesktop_date_with_and_without_timezone() {
        let ts1 = parse_tdesktop_date("19.09.2026 18:11:59 UTC+03:00").unwrap();
        let ts2 = parse_tdesktop_date("19.09.2026 15:11:59").unwrap();
        // 18:11:59 in UTC+3 is exactly 15:11:59 in UTC
        assert_eq!(ts1, ts2);
    }

    #[test]
    fn test_nested_entities_with_newlines_and_escapes() {
        let html_fragment =
            r#"<div class="text">Hello <b>world<br><i>nested &amp; special</i></b> end</div>"#;
        let doc = Html::parse_fragment(html_fragment);
        let sel = Selector::parse(".text").unwrap();
        let t_el = doc.select(&sel).next().unwrap();

        let mut emitted = String::new();
        let mut u16_len = 0;
        let mut entities = Vec::new();

        traverse_text_element(t_el, &mut emitted, &mut u16_len, &mut entities);

        assert_eq!(emitted, "Hello world\nnested & special end");
        assert_eq!(u16_len, emitted.encode_utf16().count());

        // Check bold entity covers "world\nnested & special"
        let bold = entities
            .iter()
            .find(|e| matches!(e.kind, ImportEntityKind::Bold))
            .unwrap();
        let u16_vec: Vec<u16> = emitted.encode_utf16().collect();
        let bold_slice =
            String::from_utf16(&u16_vec[bold.offset_utf16..bold.offset_utf16 + bold.length_utf16])
                .unwrap();
        assert_eq!(bold_slice, "world\nnested & special");

        // Check italic entity covers "nested & special"
        let italic = entities
            .iter()
            .find(|e| matches!(e.kind, ImportEntityKind::Italic))
            .unwrap();
        let italic_slice = String::from_utf16(
            &u16_vec[italic.offset_utf16..italic.offset_utf16 + italic.length_utf16],
        )
        .unwrap();
        assert_eq!(italic_slice, "nested & special");
    }

    #[test]
    fn parse_tdesktop_date_divider_formats() {
        assert_eq!(
            parse_tdesktop_date_divider("23 August 2026"),
            Some(ymd_to_days(2026, 8, 23) * 86400)
        );
        assert_eq!(
            parse_tdesktop_date_divider("28 августа 2026 г."),
            Some(ymd_to_days(2026, 8, 28) * 86400)
        );
        assert_eq!(
            parse_tdesktop_date_divider("14 вересня 2026 року"),
            Some(ymd_to_days(2026, 9, 14) * 86400)
        );
        assert_eq!(
            parse_tdesktop_date_divider("2026-08-23"),
            Some(ymd_to_days(2026, 8, 23) * 86400)
        );
        assert_eq!(
            parse_tdesktop_date_divider("23.08.2026"),
            Some(ymd_to_days(2026, 8, 23) * 86400)
        );
    }
}
