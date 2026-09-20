use std::{fs, path::Path, sync::LazyLock};

use scraper::{ElementRef, Html, Node, Selector};
use tracing::debug;
use vendetta_core::ymd_to_days;
use vendetta_model::{FilterReason, MediaKind, MediaRole, MessageId, PeerId, PeerType};

use crate::{
    discovery::TDesktopChatSource,
    error::{ImportError, ImportResult},
    model::{
        ImportChat, ImportEntityKind, ImportForwardInfo, ImportMediaItem, ImportMessage,
        ImportServiceEvent, ImportTextEntity,
    },
    synthetic_id::{generate_synthetic_chat_id, generate_synthetic_sender_id},
};

struct TDesktopSelectors {
    page_header: Selector,
    message: Selector,
    date: Selector,
    from_name: Selector,
    userpic: Selector,
    reply_to: Selector,
    text: Selector,
    forwarded: Selector,
    service_details: Selector,

    photo: Selector,
    video: Selector,
    animated: Selector,
    sticker: Selector,
    voice: Selector,
    audio: Selector,
    round: Selector,
    file: Selector,
    userpic_link: Selector,
}

impl TDesktopSelectors {
    fn compile() -> ImportResult<Self> {
        let sel = |p: &str| {
            Selector::parse(p).map_err(|e| {
                ImportError::ParsingFailed(format!("Invalid CSS selector '{p}': {e:?}"))
            })
        };

        Ok(Self {
            page_header: sel(".page_header .text.bold")?,
            message: sel(".message")?,
            date: sel(".pull_right.date.details")?,
            from_name: sel(".from_name")?,
            userpic: sel(".userpic")?,
            reply_to: sel(".reply_to.details a")?,
            text: sel(".text")?,
            forwarded: sel(".forwarded.body .from_name")?,
            service_details: sel("div.body.details, div.details")?,

            photo: sel("a.photo_wrap, a.media_photo")?,
            video: sel("a.video_file_wrap")?,
            animated: sel("a.animated_wrap")?,
            sticker: sel("a.sticker_wrap")?,
            voice: sel("a.media_voice_message")?,
            audio: sel("a.media_audio_file")?,
            round: sel("a.media_video")?,
            file: sel("a.media_file, div.media_file a")?,
            userpic_link: sel("div.userpic_wrap a.userpic_link, a.userpic_link")?,
        })
    }

    fn get() -> ImportResult<&'static Self> {
        static INSTANCE: LazyLock<ImportResult<TDesktopSelectors>> =
            LazyLock::new(TDesktopSelectors::compile);

        INSTANCE
            .as_ref()
            .map_err(|e| ImportError::ParsingFailed(e.to_string()))
    }
}

pub fn parse_tdesktop_html(source: &TDesktopChatSource) -> ImportResult<ImportChat> {
    if source.files.is_empty() {
        return Err(ImportError::ParsingFailed(format!(
            "No HTML chunk files found in {}",
            source.basedir.display()
        )));
    }

    let selectors = TDesktopSelectors::get()?;
    let final_title = extract_chat_title(source, selectors)?;
    let detected_type = PeerType::User;
    let chat_id = generate_synthetic_chat_id(
        &final_title,
        detected_type,
        source.internal_discriminator.as_deref(),
    );

    let mut cluster_ctx = ClusterContext::default();
    let mut messages = Vec::new();

    for file_path in &source.files {
        let content = fs::read_to_string(file_path)?;
        let doc = Html::parse_document(&content);

        messages.extend(doc.select(&selectors.message).filter_map(|msg_el| {
            cluster_ctx.process_element(msg_el, chat_id, &source.basedir, selectors)
        }));
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

fn extract_chat_title(
    source: &TDesktopChatSource,
    selectors: &TDesktopSelectors,
) -> ImportResult<String> {
    if let Some(title) = &source.title {
        return Ok(title.clone());
    }

    let title_from_file = source
        .files
        .first()
        .map(|first_file| -> ImportResult<Option<String>> {
            let content = fs::read_to_string(first_file)?;
            let doc = Html::parse_document(&content);
            Ok(doc
                .select(&selectors.page_header)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty()))
        })
        .transpose()?
        .flatten();

    Ok(title_from_file.unwrap_or_else(|| {
        source
            .basedir
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("Chat")
            .to_string()
    }))
}

#[derive(Default)]
struct ClusterContext {
    sender_name: Option<String>,
    userpic_marker: Option<String>,
    last_seen_date: Option<i64>,
}

impl ClusterContext {
    fn process_element(
        &mut self,
        msg_el: ElementRef<'_>,
        chat_id: PeerId,
        base_dir: &Path,
        selectors: &TDesktopSelectors,
    ) -> Option<ImportMessage> {
        let elem_val = msg_el.value();
        let raw_id = elem_val
            .attr("id")
            .and_then(|id| id.strip_prefix("message"))
            .and_then(|id| id.parse::<i64>().ok())
            .unwrap_or(0);

        if raw_id <= 0 {
            let divider_text = msg_el.text().collect::<String>();
            if let Some(ts) = parse_tdesktop_date_divider(&divider_text) {
                self.last_seen_date = Some(ts);
            }
            return None;
        }

        let message_id = MessageId::new(raw_id);
        let is_service = elem_val.has_class("service", scraper::CaseSensitivity::CaseSensitive);
        let is_joined = elem_val.has_class("joined", scraper::CaseSensitivity::CaseSensitive);

        let parsed_date = msg_el
            .select(&selectors.date)
            .next()
            .and_then(|el| el.value().attr("title"))
            .and_then(parse_tdesktop_date);

        if let Some(ts) = parsed_date {
            self.last_seen_date = Some(ts);
        }

        let date = parsed_date
            .or(self.last_seen_date)
            .unwrap_or_else(vendetta_core::now_unix_secs);

        if is_service {
            let service_text = msg_el
                .select(&selectors.service_details)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| msg_el.text().collect::<String>().trim().to_string());

            let media = extract_media_items(msg_el, base_dir, selectors);

            return Some(ImportMessage {
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
        }

        if !is_joined {
            if let Some(name) = msg_el
                .select(&selectors.from_name)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
            {
                self.sender_name = Some(name);
            }

            self.userpic_marker = msg_el.select(&selectors.userpic).next().and_then(|up_el| {
                up_el
                    .value()
                    .classes()
                    .find(|c| c.starts_with("userpic"))
                    .map(ToString::to_string)
                    .or_else(|| {
                        let initials = up_el.text().collect::<String>().trim().to_string();
                        (!initials.is_empty()).then_some(initials)
                    })
            });
        }

        let sender_name = self.sender_name.clone();
        let sender_id = sender_name.as_ref().map(|name| {
            generate_synthetic_sender_id(chat_id, name, self.userpic_marker.as_deref())
        });

        let reply_to_message_id = msg_el
            .select(&selectors.reply_to)
            .next()
            .and_then(|el| el.value().attr("href"))
            .and_then(extract_reply_id);

        let forward_info = msg_el
            .select(&selectors.forwarded)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|name| !name.is_empty())
            .map(|name| ImportForwardInfo {
                from_name: Some(name),
                date: None,
            });

        let (text, entities) = msg_el
            .select(&selectors.text)
            .next()
            .map(parse_message_text)
            .unwrap_or_default();

        let media = extract_media_items(msg_el, base_dir, selectors);

        Some(ImportMessage {
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
        })
    }
}

#[derive(Default)]
struct TextAccumulator {
    text: String,
    utf16_len: usize,
    entities: Vec<ImportTextEntity>,
}

impl TextAccumulator {
    fn push_text(&mut self, raw: &str) {
        let slice = if self.text.is_empty() {
            raw.trim_start_matches(['\r', '\n'])
        } else {
            raw
        };

        if !slice.is_empty() {
            self.utf16_len += slice.encode_utf16().count();
            self.text.push_str(slice);
        }
    }

    fn push_line_break(&mut self) {
        if !self.text.is_empty() {
            self.text.push('\n');
            self.utf16_len += 1;
        }
    }

    fn finish(mut self) -> (Option<String>, Vec<ImportTextEntity>) {
        let trimmed = self.text.trim_end_matches(['\r', '\n', ' ']);
        let trimmed_len_u16 = trimmed.encode_utf16().count();

        if trimmed_len_u16 < self.utf16_len {
            self.text.truncate(trimmed.len());
            for ent in &mut self.entities {
                if ent.offset_utf16 >= trimmed_len_u16 {
                    ent.length_utf16 = 0;
                } else if ent.offset_utf16 + ent.length_utf16 > trimmed_len_u16 {
                    ent.length_utf16 = trimmed_len_u16 - ent.offset_utf16;
                }
            }
            self.entities.retain(|e| e.length_utf16 > 0);
        }

        let text = (!self.text.is_empty()).then_some(self.text);
        (text, self.entities)
    }
}

fn parse_message_text(el: ElementRef<'_>) -> (Option<String>, Vec<ImportTextEntity>) {
    let mut acc = TextAccumulator::default();
    traverse_text_node(el, &mut acc);
    acc.finish()
}

fn traverse_text_node(el: ElementRef<'_>, acc: &mut TextAccumulator) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                let decoded = decode_html_entities(&t.text);
                acc.push_text(&decoded);
            }
            Node::Element(elem) => {
                if elem.name() == "br" {
                    acc.push_line_break();
                    continue;
                }

                let entity_kind = match elem.name() {
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
                        let href = elem.attr("href").unwrap_or_default().to_string();
                        Some(ImportEntityKind::TextUrl(href))
                    }
                    "span" if elem.classes().any(|c| c == "spoiler") => {
                        Some(ImportEntityKind::Spoiler)
                    }
                    "blockquote" => Some(ImportEntityKind::Blockquote),
                    _ => None,
                };

                if let Some(child_ref) = ElementRef::wrap(child) {
                    let start_u16 = acc.utf16_len;
                    traverse_text_node(child_ref, acc);
                    let end_u16 = acc.utf16_len;

                    if let Some(kind) = entity_kind {
                        let len_u16 = end_u16 - start_u16;
                        if len_u16 > 0 {
                            acc.entities.push(ImportTextEntity {
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

pub fn decode_html_entities(mut input: &str) -> String {
    let mut out = String::with_capacity(input.len());

    while let Some(amp_pos) = input.find('&') {
        out.push_str(&input[..amp_pos]);
        let rest = &input[amp_pos + 1..];

        if let Some(semi_pos) = rest.find(';')
            && semi_pos <= 10
            && !rest[..semi_pos].contains('&')
        {
            let entity = &rest[..semi_pos];
            match entity {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" | "#39" => out.push('\''),
                "nbsp" => out.push('\u{00A0}'),
                _ => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            }
            input = &rest[semi_pos + 1..];
            continue;
        }

        out.push('&');
        input = rest;
    }

    out.push_str(input);
    out
}

fn parse_triple<A: std::str::FromStr, B: std::str::FromStr, C: std::str::FromStr>(
    s: &str,
    delim: char,
) -> Option<(A, B, C)> {
    let mut it = s.split(delim);
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    it.next().is_none().then_some((a, b, c))
}

fn parse_tz_offset(tz: &str) -> Option<i64> {
    let (sign, rest) = if let Some(pos) = tz.find('+') {
        (1, tz.get(pos + 1..)?)
    } else if let Some(pos) = tz.find('-') {
        (-1, tz.get(pos + 1..)?)
    } else {
        return Some(0);
    };

    let (oh, om) = rest.split_once(':')?;
    let hours: i64 = oh.parse().ok()?;
    let mins: i64 = om.parse().ok()?;
    Some(sign * (hours * 3600 + mins * 60))
}

pub fn parse_tdesktop_date(title: &str) -> Option<i64> {
    let mut parts = title.split_whitespace();
    let date_str = parts.next()?;
    let time_str = parts.next()?;
    let tz_str = parts.next();

    let (d, m, y) = parse_triple::<u32, u32, i32>(date_str, '.')?;
    let (h, min, sec) = parse_triple::<i64, i64, i64>(time_str, ':')?;

    let days = ymd_to_days(y, m, d);
    let local_secs = days * 86400 + h * 3600 + min * 60 + sec;
    let offset_secs = tz_str.map_or(Some(0), parse_tz_offset)?;

    Some(local_secs - offset_secs)
}

pub fn parse_tdesktop_date_divider(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    if let Some((d, m, y)) = parse_numeric_date(text)
        .filter(|&(d, m, y)| (1..=31).contains(&d) && (1..=12).contains(&m) && y >= 1970)
    {
        return Some(ymd_to_days(y, m, d) * 86400);
    }

    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| {
            !matches!(
                w.to_lowercase().as_str(),
                "de" | "г." | "г" | "р." | "р" | "года" | "року" | "d'"
            )
        })
        .collect();

    if words.len() < 3 {
        return None;
    }

    let d: u32 = words.first()?.trim_end_matches('.').parse().ok()?;
    let y: i32 = words.last()?.trim_end_matches('.').parse().ok()?;

    if !(1..=31).contains(&d) || y < 1970 {
        return None;
    }

    let month_str = words.get(1)?.to_lowercase();
    let m = match_month_name(&month_str)?;

    Some(ymd_to_days(y, m, d) * 86400)
}

fn parse_numeric_date(text: &str) -> Option<(u32, u32, i32)> {
    let parts: Vec<&str> = text.split(['-', '.', '/']).collect();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => Some((d.parse().ok()?, m.parse().ok()?, y.parse().ok()?)),
        [d, m, y] if y.len() == 4 => Some((d.parse().ok()?, m.parse().ok()?, y.parse().ok()?)),
        _ => None,
    }
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
    let after = href.get(pos + "message".len()..)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<i64>().ok().map(MessageId::new)
}

fn extract_media_items(
    msg_el: ElementRef<'_>,
    base_dir: &Path,
    selectors: &TDesktopSelectors,
) -> Vec<ImportMediaItem> {
    let is_oversized = msg_el
        .text()
        .any(|chunk| chunk.contains("exceeds maximum size") || chunk.contains("Not included"));

    if is_oversized {
        return vec![ImportMediaItem {
            source_path: None,
            file_name: None,
            kind: MediaKind::Document,
            mime_type: None,
            size_bytes: None,
            is_skipped: true,
            skip_reason: Some(FilterReason::SizeAboveMax),
            role: MediaRole::Attachment,
        }];
    }

    let media_specs = [
        (
            &selectors.photo,
            MediaKind::Photo,
            Some("image/jpeg"),
            MediaRole::Attachment,
        ),
        (
            &selectors.userpic_link,
            MediaKind::Photo,
            Some("image/jpeg"),
            MediaRole::Attachment,
        ),
        (
            &selectors.animated,
            MediaKind::Animation,
            Some("video/mp4"),
            MediaRole::Attachment,
        ),
        (
            &selectors.sticker,
            MediaKind::Sticker,
            Some("image/webp"),
            MediaRole::Sticker,
        ),
        (
            &selectors.video,
            MediaKind::Video,
            Some("video/mp4"),
            MediaRole::Attachment,
        ),
        (
            &selectors.voice,
            MediaKind::Voice,
            Some("audio/ogg"),
            MediaRole::Voice,
        ),
        (
            &selectors.audio,
            MediaKind::Audio,
            Some("audio/mp3"),
            MediaRole::Attachment,
        ),
        (
            &selectors.round,
            MediaKind::VideoNote,
            Some("video/mp4"),
            MediaRole::VideoNote,
        ),
        (
            &selectors.file,
            MediaKind::Document,
            None,
            MediaRole::Attachment,
        ),
    ];

    let mut seen_hrefs = std::collections::HashSet::new();

    media_specs
        .into_iter()
        .flat_map(|(sel, kind, mime, role)| {
            msg_el.select(sel).map(move |el| (el, kind, mime, role))
        })
        .filter_map(|(el, kind, mime, role)| {
            let href = el.value().attr("href")?;
            if !seen_hrefs.insert(href) {
                return None;
            }
            let path = base_dir.join(href);
            let name = path
                .file_name()
                .and_then(|f| f.to_str())
                .map(ToString::to_string);
            Some(ImportMediaItem {
                source_path: Some(path),
                file_name: name,
                kind,
                mime_type: mime.map(ToString::to_string),
                size_bytes: None,
                is_skipped: false,
                skip_reason: None,
                role,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tdesktop_date_with_and_without_timezone() {
        let ts1 = parse_tdesktop_date("19.09.2026 18:11:59 UTC+03:00").unwrap();
        let ts2 = parse_tdesktop_date("19.09.2026 15:11:59").unwrap();
        assert_eq!(ts1, ts2);
    }

    #[test]
    fn test_nested_entities_with_newlines_and_escapes() {
        let html_fragment =
            r#"<div class="text">Hello <b>world<br><i>nested &amp; special</i></b> end</div>"#;
        let doc = Html::parse_fragment(html_fragment);
        let sel = Selector::parse(".text").unwrap();
        let t_el = doc.select(&sel).next().unwrap();

        let (text, entities) = parse_message_text(t_el);
        let emitted = text.unwrap();

        assert_eq!(emitted, "Hello world\nnested & special end");

        let u16_vec: Vec<u16> = emitted.encode_utf16().collect();

        let bold = entities
            .iter()
            .find(|e| matches!(e.kind, ImportEntityKind::Bold))
            .unwrap();
        let bold_slice =
            String::from_utf16(&u16_vec[bold.offset_utf16..bold.offset_utf16 + bold.length_utf16])
                .unwrap();
        assert_eq!(bold_slice, "world\nnested & special");

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
