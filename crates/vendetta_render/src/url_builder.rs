use std::collections::HashSet;

use vendetta_model::{MessageId, PeerId};

use crate::entity::html_escape;

#[derive(Debug, Clone, Default)]
pub struct ArchiveUrlBuilder;

impl ArchiveUrlBuilder {
    pub fn peer_token(peer_id: PeerId) -> String {
        let raw = peer_id.raw();
        if raw < 0 {
            format!("p_neg_{}", raw.unsigned_abs())
        } else {
            format!("p_{raw}")
        }
    }

    pub fn page_file_name(page_index: usize) -> String {
        format!("page_{:05}.html", page_index + 1)
    }

    pub fn topic_page_file_name(topic_id: i32, page_index: usize) -> String {
        format!("topic_{topic_id}_page_{:05}.html", page_index + 1)
    }

    pub fn peer_dir_rel(peer_id: PeerId) -> String {
        format!("chats/{}", Self::peer_token(peer_id))
    }

    pub fn chunk_file_rel(peer_id: PeerId, page_index: usize) -> String {
        format!(
            "{}/{}",
            Self::peer_dir_rel(peer_id),
            Self::page_file_name(page_index)
        )
    }

    pub fn topic_chunk_file_rel(peer_id: PeerId, topic_id: i32, page_index: usize) -> String {
        format!(
            "{}/{}",
            Self::peer_dir_rel(peer_id),
            Self::topic_page_file_name(topic_id, page_index)
        )
    }

    pub fn message_anchor(peer_id: PeerId, message_id: MessageId) -> String {
        let raw_msg = message_id.raw();
        let msg_str = if raw_msg < 0 {
            format!("neg_{}", raw_msg.unsigned_abs())
        } else {
            raw_msg.to_string()
        };
        format!("m-{}-{}", Self::peer_token(peer_id), msg_str)
    }

    pub fn relative_url(from_depth: usize, target_rel_path: &str) -> String {
        if from_depth == 0 {
            return target_rel_path.to_string();
        }
        format!("{}{target_rel_path}", "../".repeat(from_depth))
    }

    pub fn chunk_to_chunk_url(
        from_peer: PeerId,
        target_peer: PeerId,
        target_page_index: usize,
    ) -> String {
        if from_peer == target_peer {
            Self::page_file_name(target_page_index)
        } else {
            format!(
                "../{}/{}",
                Self::peer_token(target_peer),
                Self::page_file_name(target_page_index)
            )
        }
    }

    pub fn topic_chunk_to_chunk_url(
        from_peer: PeerId,
        target_peer: PeerId,
        target_topic_id: Option<i32>,
        target_page_index: usize,
    ) -> String {
        let file_name = if let Some(tid) = target_topic_id {
            Self::topic_page_file_name(tid, target_page_index)
        } else {
            Self::page_file_name(target_page_index)
        };

        if from_peer == target_peer {
            file_name
        } else {
            format!("../{}/{}", Self::peer_token(target_peer), file_name)
        }
    }

    pub fn message_full_link(
        from_peer: Option<PeerId>,
        from_depth: usize,
        target_peer: PeerId,
        target_page_index: usize,
        target_message_id: MessageId,
    ) -> String {
        let anchor = Self::message_anchor(target_peer, target_message_id);
        if let Some(src_peer) = from_peer
            && from_depth == 2
        {
            let chunk_url = Self::chunk_to_chunk_url(src_peer, target_peer, target_page_index);
            format!("{chunk_url}#{anchor}")
        } else {
            let target_chunk = Self::chunk_file_rel(target_peer, target_page_index);
            let rel = Self::relative_url(from_depth, &target_chunk);
            format!("{rel}#{anchor}")
        }
    }

    pub fn media_url(from_depth: usize, local_rel_path: &str) -> String {
        let clean = local_rel_path.trim_start_matches('/');
        if clean.starts_with("media/") {
            Self::relative_url(from_depth, clean)
        } else {
            Self::relative_url(from_depth, &format!("media/{clean}"))
        }
    }

    pub fn root_index_url(from_depth: usize) -> String {
        Self::relative_url(from_depth, "index.html")
    }

    pub fn chat_root_url(from_depth: usize, peer_id: PeerId) -> String {
        let chunk = Self::chunk_file_rel(peer_id, 0);
        Self::relative_url(from_depth, &chunk)
    }

    pub fn topic_chat_root_url(
        from_depth: usize,
        peer_id: PeerId,
        default_topic_id: Option<i32>,
    ) -> String {
        let chunk = if let Some(tid) = default_topic_id {
            Self::topic_chunk_file_rel(peer_id, tid, 0)
        } else {
            Self::chunk_file_rel(peer_id, 0)
        };
        Self::relative_url(from_depth, &chunk)
    }

    pub fn avatar_rel_path(peer_id: PeerId) -> String {
        format!("media/avatars/{}.jpg", Self::peer_token(peer_id))
    }

    pub fn avatar_url(from_depth: usize, peer_id: PeerId) -> String {
        Self::relative_url(from_depth, &Self::avatar_rel_path(peer_id))
    }
}

pub fn render_avatar_markup(
    peer_id: Option<PeerId>,
    name: &str,
    from_depth: usize,
    is_large: bool,
    avatar_class: &str,
    available_avatars: &HashSet<PeerId>,
) -> String {
    let extra_cls = if is_large { " avatar-lg" } else { "" };
    let cls = format!("{avatar_class}{extra_cls}");

    if let Some(pid) = peer_id
        && available_avatars.contains(&pid)
    {
        let img_src = ArchiveUrlBuilder::avatar_url(from_depth, pid);
        let alt = html_escape(name);
        return format!(
            "<div class=\"{cls}\"><img src=\"{img_src}\" alt=\"{alt}\" class=\"avatar-img\"></div>"
        );
    }

    let initial = name.chars().next().unwrap_or('?');
    let text = html_escape(&initial.to_string());
    format!("<div class=\"{cls}\"><span class=\"avatar-text\">{text}</span></div>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_builder_formats_positive_and_negative_peer_tokens() {
        assert_eq!(ArchiveUrlBuilder::peer_token(PeerId::new(12345)), "p_12345");
        assert_eq!(
            ArchiveUrlBuilder::peer_token(PeerId::new(-100123456789)),
            "p_neg_100123456789"
        );
    }

    #[test]
    fn url_builder_formats_message_anchors() {
        assert_eq!(
            ArchiveUrlBuilder::message_anchor(PeerId::new(100), MessageId::new(42)),
            "m-p_100-42"
        );
        assert_eq!(
            ArchiveUrlBuilder::message_anchor(PeerId::new(-100), MessageId::new(99)),
            "m-p_neg_100-99"
        );
    }

    #[test]
    fn url_builder_builds_relative_urls() {
        assert_eq!(
            ArchiveUrlBuilder::relative_url(0, "assets/css/main.css"),
            "assets/css/main.css"
        );
        assert_eq!(
            ArchiveUrlBuilder::relative_url(1, "assets/css/main.css"),
            "../assets/css/main.css"
        );
        assert_eq!(
            ArchiveUrlBuilder::relative_url(2, "assets/css/main.css"),
            "../../assets/css/main.css"
        );
    }

    #[test]
    fn url_builder_builds_chunk_navigation_links() {
        let p1 = PeerId::new(100);
        let p2 = PeerId::new(200);

        assert_eq!(
            ArchiveUrlBuilder::chunk_to_chunk_url(p1, p1, 1),
            "page_00002.html"
        );

        assert_eq!(
            ArchiveUrlBuilder::chunk_to_chunk_url(p1, p2, 0),
            "../p_200/page_00001.html"
        );
    }

    #[test]
    fn url_builder_builds_full_message_links() {
        let p1 = PeerId::new(100);
        let p2 = PeerId::new(200);

        assert_eq!(
            ArchiveUrlBuilder::message_full_link(None, 0, p1, 0, MessageId::new(10)),
            "chats/p_100/page_00001.html#m-p_100-10"
        );

        assert_eq!(
            ArchiveUrlBuilder::message_full_link(Some(p1), 2, p1, 1, MessageId::new(25)),
            "page_00002.html#m-p_100-25"
        );

        assert_eq!(
            ArchiveUrlBuilder::message_full_link(Some(p1), 2, p2, 0, MessageId::new(50)),
            "../p_200/page_00001.html#m-p_200-50"
        );
    }
}
