use std::sync::Arc;
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};

use grammers_tl_types::{self as tl, Deserializable, Serializable};
use vendetta_model::PeerId;
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::error::{MediaEngineError, MediaEngineResult};
use crate::storage_layout::StorageLayoutManager;

#[derive(Debug, Clone)]
pub struct PeerAvatarLocation {
    pub peer_id: PeerId,
    pub photo_id: i64,
    pub dc_id: i32,
    pub location_tl: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarDownloadStatus {
    Downloaded(usize),
    AlreadyExists,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Default)]
pub struct AvatarSyncSummary {
    pub total_peers: usize,
    pub avatars_discovered: usize,
    pub downloaded: usize,
    pub already_existed: usize,
    pub unavailable: usize,
    pub failed: usize,
    pub total_bytes: u64,
}

pub fn peer_avatar_file_name(peer_id: PeerId) -> String {
    let raw = peer_id.raw();
    if raw < 0 {
        format!("p_neg_{}.jpg", raw.unsigned_abs())
    } else {
        format!("p_{raw}.jpg")
    }
}

fn make_location(
    peer_id: PeerId,
    photo_id: i64,
    dc_id: i32,
    peer: tl::enums::InputPeer,
) -> PeerAvatarLocation {
    let location = tl::enums::InputFileLocation::InputPeerPhotoFileLocation(
        tl::types::InputPeerPhotoFileLocation {
            big: false,
            peer,
            photo_id,
        },
    );
    PeerAvatarLocation {
        peer_id,
        photo_id,
        dc_id,
        location_tl: location.to_bytes(),
    }
}

pub fn extract_peer_avatar_location(peer_id: PeerId, raw_tl: &[u8]) -> Option<PeerAvatarLocation> {
    if let Ok(tl::enums::User::User(u)) = tl::enums::User::from_bytes(raw_tl) {
        let access_hash = u.access_hash.filter(|&h| h != 0)?;
        if let Some(tl::enums::UserProfilePhoto::Photo(p)) = u.photo {
            return Some(make_location(
                peer_id,
                p.photo_id,
                p.dc_id,
                tl::enums::InputPeer::User(tl::types::InputPeerUser {
                    user_id: u.id,
                    access_hash,
                }),
            ));
        }
        return None;
    }

    if let Ok(chat) = tl::enums::Chat::from_bytes(raw_tl) {
        match chat {
            tl::enums::Chat::Chat(c) => {
                if let tl::enums::ChatPhoto::Photo(p) = c.photo {
                    return Some(make_location(
                        peer_id,
                        p.photo_id,
                        p.dc_id,
                        tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: c.id }),
                    ));
                }
            }
            tl::enums::Chat::Channel(c) => {
                let access_hash = c.access_hash.filter(|&h| h != 0)?;
                if let tl::enums::ChatPhoto::Photo(p) = c.photo {
                    return Some(make_location(
                        peer_id,
                        p.photo_id,
                        p.dc_id,
                        tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                            channel_id: c.id,
                            access_hash,
                        }),
                    ));
                }
            }
            _ => {}
        }
    }

    None
}

pub async fn download_single_avatar(
    adapter: &Arc<dyn TelegramAdapter>,
    storage_layout: &StorageLayoutManager,
    avatar_loc: &PeerAvatarLocation,
) -> MediaEngineResult<AvatarDownloadStatus> {
    let avatars_dir = storage_layout.avatars_dir();
    fs::create_dir_all(&avatars_dir).await?;

    let final_path = avatars_dir.join(peer_avatar_file_name(avatar_loc.peer_id));

    if let Ok(meta) = fs::metadata(&final_path).await
        && meta.len() > 0
    {
        return Ok(AvatarDownloadStatus::AlreadyExists);
    }

    let temp_path = storage_layout.temp_part_path(&format!(
        "avatar_{}_{}",
        avatar_loc.peer_id.raw(),
        avatar_loc.photo_id
    ));

    let mut file = File::create(&temp_path).await?;
    let mut offset: i64 = 0;
    const CHUNK_LIMIT: i32 = 131_072;
    let mut total_downloaded: usize = 0;

    loop {
        let chunk = match adapter
            .download_file_chunk(
                &avatar_loc.location_tl,
                avatar_loc.dc_id,
                offset,
                CHUNK_LIMIT,
            )
            .await
        {
            Ok(bytes) => bytes,
            Err(err) => {
                let _ = fs::remove_file(&temp_path).await;
                return Err(MediaEngineError::Adapter(err));
            }
        };

        if chunk.is_empty() {
            break;
        }

        let len = chunk.len();
        file.write_all(&chunk).await?;

        total_downloaded += len;
        offset += len as i64;

        if len < CHUNK_LIMIT as usize {
            break;
        }
    }

    file.flush().await?;
    drop(file);

    if total_downloaded == 0 {
        let _ = fs::remove_file(&temp_path).await;
        return Ok(AvatarDownloadStatus::Unavailable);
    }

    fs::rename(&temp_path, &final_path).await?;
    Ok(AvatarDownloadStatus::Downloaded(total_downloaded))
}

pub async fn sync_all_peer_avatars<F>(
    db: &ArchiveDb,
    adapter: &Arc<dyn TelegramAdapter>,
    storage_layout: &StorageLayoutManager,
    mut progress_cb: F,
) -> MediaEngineResult<AvatarSyncSummary>
where
    F: FnMut(&AvatarSyncSummary),
{
    let peers = db.list_peers()?;

    let mut summary = AvatarSyncSummary {
        total_peers: peers.len(),
        ..Default::default()
    };

    let locations: Vec<_> = peers
        .iter()
        .filter_map(|peer| {
            let raw_tl = peer.raw_tl.as_ref()?;
            extract_peer_avatar_location(peer.peer_id, raw_tl)
        })
        .collect();

    summary.avatars_discovered = locations.len();
    summary.unavailable = summary
        .total_peers
        .saturating_sub(summary.avatars_discovered);
    progress_cb(&summary);

    for loc in locations {
        match download_single_avatar(adapter, storage_layout, &loc).await {
            Ok(AvatarDownloadStatus::Downloaded(bytes)) => {
                summary.downloaded += 1;
                summary.total_bytes += bytes as u64;
            }
            Ok(AvatarDownloadStatus::AlreadyExists) => {
                summary.already_existed += 1;
            }
            Ok(AvatarDownloadStatus::Unavailable) => {
                summary.unavailable += 1;
            }
            Ok(AvatarDownloadStatus::Failed) | Err(_) => {
                summary.failed += 1;
            }
        }
        progress_cb(&summary);
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_avatar_file_name() {
        assert_eq!(peer_avatar_file_name(PeerId::new(12345)), "p_12345.jpg");
        assert_eq!(
            peer_avatar_file_name(PeerId::new(-100123456789)),
            "p_neg_100123456789.jpg"
        );
    }

    #[test]
    fn user_avatar_extraction_strictly_validates_access_hash() {
        let user_with_hash = tl::types::User {
            is_self: false,
            contact: false,
            mutual_contact: false,
            deleted: false,
            bot: false,
            bot_chat_history: false,
            bot_nochats: false,
            verified: false,
            restricted: false,
            min: false,
            bot_inline_geo: false,
            support: false,
            scam: false,
            apply_min_photo: false,
            fake: false,
            bot_attach_menu: false,
            premium: false,
            attach_menu_enabled: false,
            bot_can_edit: false,
            close_friend: false,
            stories_hidden: false,
            stories_unavailable: false,
            contact_require_premium: false,
            bot_business: false,
            bot_has_main_app: false,
            bot_forum_view: false,
            bot_forum_can_manage_topics: false,
            bot_can_manage_bots: false,
            bot_guestchat: false,
            bot_guard: false,
            id: 42,
            access_hash: Some(999888),
            first_name: Some("Alice".to_string()),
            last_name: None,
            username: None,
            phone: None,
            photo: Some(tl::enums::UserProfilePhoto::Photo(
                tl::types::UserProfilePhoto {
                    has_video: false,
                    personal: false,
                    photo_id: 1001,
                    dc_id: 2,
                    stripped_thumb: None,
                },
            )),
            status: None,
            bot_info_version: None,
            restriction_reason: None,
            bot_inline_placeholder: None,
            lang_code: None,
            emoji_status: None,
            usernames: None,
            stories_max_id: None,
            color: None,
            profile_color: None,
            bot_active_users: None,
            bot_verification_icon: None,
            send_paid_messages_stars: None,
            linked_community_id: None,
        };

        let raw_tl = tl::enums::User::User(user_with_hash.clone()).to_bytes();
        let loc = extract_peer_avatar_location(PeerId::new(42), &raw_tl).unwrap();
        assert_eq!(loc.photo_id, 1001);
        assert_eq!(loc.dc_id, 2);

        let mut user_no_hash = user_with_hash.clone();
        user_no_hash.access_hash = None;
        let raw_no_hash = tl::enums::User::User(user_no_hash).to_bytes();
        assert!(extract_peer_avatar_location(PeerId::new(42), &raw_no_hash).is_none());

        let mut user_zero_hash = user_with_hash.clone();
        user_zero_hash.access_hash = Some(0);
        let raw_zero_hash = tl::enums::User::User(user_zero_hash).to_bytes();
        assert!(extract_peer_avatar_location(PeerId::new(42), &raw_zero_hash).is_none());

        let mut user_empty_photo = user_with_hash.clone();
        user_empty_photo.photo = Some(tl::enums::UserProfilePhoto::Empty);
        let raw_empty_photo = tl::enums::User::User(user_empty_photo).to_bytes();
        assert!(extract_peer_avatar_location(PeerId::new(42), &raw_empty_photo).is_none());
    }

    #[test]
    fn channel_avatar_extraction_strictly_validates_access_hash() {
        let channel_with_hash = tl::types::Channel {
            creator: false,
            left: false,
            broadcast: false,
            verified: false,
            megagroup: true,
            restricted: false,
            signatures: false,
            min: false,
            scam: false,
            has_link: false,
            has_geo: false,
            slowmode_enabled: false,
            call_active: false,
            call_not_empty: false,
            fake: false,
            gigagroup: false,
            noforwards: false,
            join_to_send: false,
            join_request: false,
            forum: false,
            stories_hidden: false,
            stories_hidden_min: false,
            stories_unavailable: false,
            signature_profiles: false,
            autotranslation: false,
            broadcast_messages_allowed: false,
            monoforum: false,
            forum_tabs: false,
            id: 888,
            access_hash: Some(777111),
            title: "Supergroup".to_string(),
            username: None,
            photo: tl::enums::ChatPhoto::Photo(tl::types::ChatPhoto {
                has_video: false,
                photo_id: 2002,
                dc_id: 4,
                stripped_thumb: None,
            }),
            date: 1000,
            restriction_reason: None,
            admin_rights: None,
            banned_rights: None,
            default_banned_rights: None,
            participants_count: None,
            usernames: None,
            stories_max_id: None,
            color: None,
            profile_color: None,
            emoji_status: None,
            level: None,
            subscription_until_date: None,
            bot_verification_icon: None,
            send_paid_messages_stars: None,
            linked_monoforum_id: None,
            linked_community_id: None,
        };

        let raw_tl = tl::enums::Chat::Channel(channel_with_hash.clone()).to_bytes();
        let loc = extract_peer_avatar_location(PeerId::new(-100888), &raw_tl).unwrap();
        assert_eq!(loc.photo_id, 2002);
        assert_eq!(loc.dc_id, 4);

        let mut chan_no_hash = channel_with_hash.clone();
        chan_no_hash.access_hash = None;
        let raw_no_hash = tl::enums::Chat::Channel(chan_no_hash).to_bytes();
        assert!(extract_peer_avatar_location(PeerId::new(-100888), &raw_no_hash).is_none());

        let mut chan_zero_hash = channel_with_hash.clone();
        chan_zero_hash.access_hash = Some(0);
        let raw_zero_hash = tl::enums::Chat::Channel(chan_zero_hash).to_bytes();
        assert!(extract_peer_avatar_location(PeerId::new(-100888), &raw_zero_hash).is_none());
    }
}
