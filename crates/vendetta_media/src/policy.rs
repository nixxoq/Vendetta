use std::path::Path;
use vendetta_model::{
    FilterDecision, FilterReason, MediaFilterPolicy, MediaKind, MediaRecord, PeerId,
};

pub struct MediaPolicyEvaluator;

impl MediaPolicyEvaluator {
    pub fn evaluate(
        policy: &MediaFilterPolicy,
        record: &MediaRecord,
        peer_id: Option<PeerId>,
    ) -> (FilterDecision, Option<FilterReason>) {
        if let Some(target_peers) = &policy.target_peers
            && let Some(pid) = peer_id
            && !target_peers.contains(&pid)
        {
            return (FilterDecision::Skip, Some(FilterReason::Manual));
        }

        let type_allowed = match record.kind {
            MediaKind::Photo => policy.allow_photos,
            MediaKind::Video => policy.allow_videos,
            MediaKind::Document => policy.allow_documents,
            MediaKind::Audio => policy.allow_audio,
            MediaKind::Voice => policy.allow_voice,
            MediaKind::Sticker => policy.allow_stickers,
            MediaKind::Animation => policy.allow_animations,
            MediaKind::VideoNote => policy.allow_video_notes,
            MediaKind::Thumbnail | MediaKind::Other => true,
        };

        if !type_allowed {
            return (FilterDecision::Skip, Some(FilterReason::TypeExcluded));
        }

        if let Some(size) = record.size_bytes {
            if policy.min_size_bytes.is_some_and(|min| size < min) {
                return (FilterDecision::Skip, Some(FilterReason::SizeBelowMin));
            }
            if policy.max_size_bytes.is_some_and(|max| size > max) {
                return (FilterDecision::Skip, Some(FilterReason::SizeAboveMax));
            }
        }

        if let Some(mime) = &record.mime_type
            && is_value_excluded(
                policy.blocked_mime_types.as_deref(),
                policy.allowed_mime_types.as_deref(),
                mime,
            )
        {
            return (FilterDecision::Skip, Some(FilterReason::MimeExcluded));
        }

        if let Some(name) = &record.file_name
            && let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str())
            && is_value_excluded(
                policy.blocked_extensions.as_deref(),
                policy.allowed_extensions.as_deref(),
                ext,
            )
        {
            return (FilterDecision::Skip, Some(FilterReason::ExtExcluded));
        }

        (FilterDecision::Allow, None)
    }
}

fn is_value_excluded(blocked: Option<&[String]>, allowed: Option<&[String]>, value: &str) -> bool {
    if let Some(blocked) = blocked
        && blocked.iter().any(|b| value.eq_ignore_ascii_case(b))
    {
        return true;
    }
    if let Some(allowed) = allowed
        && !allowed.iter().any(|a| value.eq_ignore_ascii_case(a))
    {
        return true;
    }
    false
}
