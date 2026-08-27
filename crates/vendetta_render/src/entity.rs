use grammers_tl_types as tl;

pub fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

pub fn sanitize_href(url: &str) -> String {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return "#blocked-unsafe-url".to_string();
    }

    const SAFE_PREFIXES: &[&str] = &[
        "http://", "https://", "mailto:", "tel:", "tg:", "#", "/", "./", "../",
    ];

    if SAFE_PREFIXES.iter().any(|prefix| lower.starts_with(prefix)) {
        html_escape(trimmed)
    } else {
        format!("https://{}", html_escape(trimmed))
    }
}

struct Utf16ByteMapper {
    utf16_to_byte: Vec<usize>,
}

impl Utf16ByteMapper {
    fn new(text: &str) -> Self {
        let mut utf16_to_byte = Vec::with_capacity(text.len() + 1);
        let mut current_byte = 0;
        for c in text.chars() {
            utf16_to_byte.push(current_byte);
            if c.len_utf16() == 2 {
                utf16_to_byte.push(current_byte);
            }
            current_byte += c.len_utf8();
        }
        utf16_to_byte.push(current_byte);
        Self { utf16_to_byte }
    }

    fn map_range(
        &self,
        utf16_offset: i32,
        utf16_length: i32,
        max_byte_len: usize,
    ) -> Option<(usize, usize)> {
        if utf16_offset < 0 || utf16_length <= 0 {
            return None;
        }
        let start_u16 = utf16_offset as usize;
        let end_u16 = start_u16.saturating_add(utf16_length as usize);

        let start_byte = *self.utf16_to_byte.get(start_u16)?;
        let end_byte = self
            .utf16_to_byte
            .get(end_u16)
            .copied()
            .unwrap_or(max_byte_len);

        if start_byte < end_byte && end_byte <= max_byte_len {
            Some((start_byte, end_byte))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TagType {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Pre(Option<String>),
    Spoiler,
    Link(String),
    Mention,
    Hashtag,
    BotCommand,
    Email(String),
    Phone(String),
    CustomEmoji(i64),
    Blockquote { collapsed: bool },
}

impl TagType {
    fn open_tag(&self) -> String {
        match self {
            Self::Bold => "<strong>".to_string(),
            Self::Italic => "<em>".to_string(),
            Self::Underline => "<u>".to_string(),
            Self::Strike => "<s>".to_string(),
            Self::Code => "<code class=\"inline-code\">".to_string(),
            Self::Pre(lang) => {
                if let Some(l) = lang {
                    format!(
                        "<pre><code class=\"code-block\" data-lang=\"{}\">",
                        html_escape(l)
                    )
                } else {
                    "<pre><code class=\"code-block\">".to_string()
                }
            }
            Self::Spoiler => "<span class=\"tg-spoiler spoiler\">".to_string(),
            Self::Link(url) => format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">",
                sanitize_href(url)
            ),
            Self::Mention => "<span class=\"mention\">".to_string(),
            Self::Hashtag => "<span class=\"hashtag\">".to_string(),
            Self::BotCommand => "<span class=\"bot-command\">".to_string(),
            Self::Email(email) => format!("<a href=\"mailto:{}\">", html_escape(email)),
            Self::Phone(phone) => format!("<a href=\"tel:{}\">", html_escape(phone)),
            Self::CustomEmoji(doc_id) => {
                format!("<span class=\"custom-emoji\" data-doc-id=\"{doc_id}\">")
            }
            Self::Blockquote { collapsed } => {
                if *collapsed {
                    "<blockquote class=\"tg-blockquote tg-blockquote-collapsed\" data-collapsed=\"true\">".to_string()
                } else {
                    "<blockquote class=\"tg-blockquote\">".to_string()
                }
            }
        }
    }

    fn close_tag(&self) -> &'static str {
        match self {
            Self::Bold => "</strong>",
            Self::Italic => "</em>",
            Self::Underline => "</u>",
            Self::Strike => "</s>",
            Self::Code => "</code>",
            Self::Pre(_) => "</code></pre>",
            Self::Spoiler => "</span>",
            Self::Link(_) => "</a>",
            Self::Mention | Self::Hashtag | Self::BotCommand | Self::CustomEmoji(_) => "</span>",
            Self::Email(_) | Self::Phone(_) => "</a>",
            Self::Blockquote { .. } => "</blockquote>",
        }
    }
}

struct EntityRange {
    start: usize,
    end: usize,
    tag: TagType,
}

fn escape_newlines(text: &str) -> String {
    html_escape(text).replace('\n', "<br>")
}

pub fn render_formatted_text(raw_text: &str, entities_json: Option<&str>) -> String {
    if raw_text.is_empty() {
        return String::new();
    }

    let Some(json_str) = entities_json else {
        return escape_newlines(raw_text);
    };

    let Ok(entities) = serde_json::from_str::<Vec<tl::enums::MessageEntity>>(json_str) else {
        return escape_newlines(raw_text);
    };

    if entities.is_empty() {
        return escape_newlines(raw_text);
    }

    let mapper = Utf16ByteMapper::new(raw_text);
    let mut ranges = Vec::new();

    for ent in entities {
        let (u16_off, u16_len, tag) = match ent {
            tl::enums::MessageEntity::Bold(b) => (b.offset, b.length, TagType::Bold),
            tl::enums::MessageEntity::Italic(i) => (i.offset, i.length, TagType::Italic),
            tl::enums::MessageEntity::Underline(u) => (u.offset, u.length, TagType::Underline),
            tl::enums::MessageEntity::Strike(s) => (s.offset, s.length, TagType::Strike),
            tl::enums::MessageEntity::Code(c) => (c.offset, c.length, TagType::Code),
            tl::enums::MessageEntity::Pre(p) => {
                let lang = if p.language.is_empty() {
                    None
                } else {
                    Some(p.language)
                };
                (p.offset, p.length, TagType::Pre(lang))
            }
            tl::enums::MessageEntity::Spoiler(s) => (s.offset, s.length, TagType::Spoiler),
            tl::enums::MessageEntity::TextUrl(t) => (t.offset, t.length, TagType::Link(t.url)),
            tl::enums::MessageEntity::Url(u) => {
                if let Some((s_byte, e_byte)) = mapper.map_range(u.offset, u.length, raw_text.len())
                {
                    let extracted_url = &raw_text[s_byte..e_byte];
                    (u.offset, u.length, TagType::Link(extracted_url.to_string()))
                } else {
                    continue;
                }
            }
            tl::enums::MessageEntity::Mention(m) => (m.offset, m.length, TagType::Mention),
            tl::enums::MessageEntity::Hashtag(h) => (h.offset, h.length, TagType::Hashtag),
            tl::enums::MessageEntity::BotCommand(b) => (b.offset, b.length, TagType::BotCommand),
            tl::enums::MessageEntity::Email(e) => {
                if let Some((s_byte, e_byte)) = mapper.map_range(e.offset, e.length, raw_text.len())
                {
                    let extracted = &raw_text[s_byte..e_byte];
                    (e.offset, e.length, TagType::Email(extracted.to_string()))
                } else {
                    continue;
                }
            }
            tl::enums::MessageEntity::Phone(p) => {
                if let Some((s_byte, e_byte)) = mapper.map_range(p.offset, p.length, raw_text.len())
                {
                    let extracted = &raw_text[s_byte..e_byte];
                    (p.offset, p.length, TagType::Phone(extracted.to_string()))
                } else {
                    continue;
                }
            }
            tl::enums::MessageEntity::CustomEmoji(ce) => {
                (ce.offset, ce.length, TagType::CustomEmoji(ce.document_id))
            }
            tl::enums::MessageEntity::Blockquote(bq) => (
                bq.offset,
                bq.length,
                TagType::Blockquote {
                    collapsed: bq.collapsed,
                },
            ),
            _ => continue,
        };

        if let Some((start, end)) = mapper.map_range(u16_off, u16_len, raw_text.len()) {
            ranges.push(EntityRange { start, end, tag });
        }
    }

    if ranges.is_empty() {
        return escape_newlines(raw_text);
    }

    ranges.sort_by(|a, b| {
        if a.start != b.start {
            a.start.cmp(&b.start)
        } else {
            b.end.cmp(&a.end)
        }
    });

    render_with_spans(raw_text, &ranges)
}

fn render_with_spans(text: &str, ranges: &[EntityRange]) -> String {
    let mut result = String::with_capacity(text.len() + 64);
    let mut cursor = 0;

    for range in ranges {
        if range.start > cursor {
            let slice = &text[cursor..range.start];
            result.push_str(&escape_newlines(slice));
            cursor = range.start;
        }

        if range.start < cursor {
            continue;
        }

        result.push_str(&range.tag.open_tag());
        let slice = &text[range.start..range.end];
        result.push_str(&escape_newlines(slice));
        result.push_str(range.tag.close_tag());
        cursor = range.end;
    }

    if cursor < text.len() {
        let slice = &text[cursor..];
        result.push_str(&escape_newlines(slice));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_escapes_special_characters() {
        assert_eq!(
            html_escape("<script>alert('xss' & \"test\")</script>"),
            "&lt;script&gt;alert(&#39;xss&#39; &amp; &quot;test&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn sanitize_href_disallows_dangerous_protocols() {
        assert_eq!(sanitize_href("javascript:alert(1)"), "#blocked-unsafe-url");
        assert_eq!(sanitize_href("JAVASCRIPT:alert(1)"), "#blocked-unsafe-url");
        assert_eq!(
            sanitize_href("data:text/html,<script>"),
            "#blocked-unsafe-url"
        );
        assert_eq!(
            sanitize_href("https://example.com/page?a=1&b=2"),
            "https://example.com/page?a=1&amp;b=2"
        );
    }

    #[test]
    fn entity_formatting_aligns_with_utf16_code_units() {
        let text = "👋 Hello!";
        let entities = vec![tl::enums::MessageEntity::Bold(
            tl::types::MessageEntityBold {
                offset: 3,
                length: 5,
            },
        )];
        let json = serde_json::to_string(&entities).unwrap();
        let rendered = render_formatted_text(text, Some(&json));
        assert_eq!(rendered, "👋 <strong>Hello</strong>!");
    }

    #[test]
    fn text_url_entity_renders_anchor_tag() {
        let text = "Visit Telegram website";
        let entities = vec![tl::enums::MessageEntity::TextUrl(
            tl::types::MessageEntityTextUrl {
                offset: 6,
                length: 8,
                url: "https://telegram.org".to_string(),
            },
        )];
        let json = serde_json::to_string(&entities).unwrap();
        let rendered = render_formatted_text(text, Some(&json));
        assert_eq!(
            rendered,
            "Visit <a href=\"https://telegram.org\" target=\"_blank\" rel=\"noopener noreferrer\">Telegram</a> website"
        );
    }
}
