use std::{collections::HashSet, fs, path::Path};

use vendetta_model::PeerId;

use crate::{
    error::RenderResult,
    layout::dialog::{DialogPageContext, render_dialog_page},
    message::group_messages_into_render_items,
    model::{ExportOptions, RenderItem, RenderMessage, RenderPeer, RenderTopic},
    navigation::DateNavigator,
    reply::ReplyLocationMap,
    url_builder::ArchiveUrlBuilder,
};

fn build_date_navigator(msgs: &[RenderMessage], chunk_size: usize, enabled: bool) -> DateNavigator {
    let mut nav = DateNavigator::new();
    if enabled {
        for (msg_idx, m) in msgs.iter().enumerate() {
            nav.record_message_date(m.date, msg_idx / chunk_size);
        }
    }
    nav
}

fn chunk_continuation_gids(
    msgs: &[RenderMessage],
    chunk_start: usize,
    chunk_end: usize,
    page_idx: usize,
    total_pages: usize,
) -> (Option<i64>, Option<i64>) {
    let raw_chunk = if chunk_start < msgs.len() {
        &msgs[chunk_start..chunk_end]
    } else {
        &[]
    };

    let first_gid = raw_chunk.first().and_then(|m| m.grouped_id);
    let last_gid = raw_chunk.last().and_then(|m| m.grouped_id);

    let cont_prev_gid = if page_idx > 0 && first_gid.is_some() && chunk_start > 0 {
        msgs.get(chunk_start - 1)
            .and_then(|m| m.grouped_id)
            .filter(|&gid| Some(gid) == first_gid)
    } else {
        None
    };

    let cont_next_gid = if (page_idx + 1) < total_pages && last_gid.is_some() {
        msgs.get(chunk_end)
            .and_then(|m| m.grouped_id)
            .filter(|&gid| Some(gid) == last_gid)
    } else {
        None
    };

    (cont_prev_gid, cont_next_gid)
}

pub fn render_topic_scoped_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    location_map: &ReplyLocationMap,
    options: &ExportOptions,
    available_avatars: &HashSet<PeerId>,
) -> RenderResult<usize> {
    let mut total_chunks = 0;

    for topic in &current_peer.topics {
        let topic_msgs: Vec<RenderMessage> = all_render_messages
            .iter()
            .filter(|m| {
                let tid = location_map
                    .get_location(&m.key)
                    .and_then(|loc| loc.1)
                    .unwrap_or(1);
                tid == topic.topic_id
            })
            .cloned()
            .collect();

        let total_t_msgs = topic_msgs.len();
        let total_pages = if total_t_msgs == 0 {
            1
        } else {
            total_t_msgs.div_ceil(options.chunk_size)
        };

        let date_navigator =
            build_date_navigator(&topic_msgs, options.chunk_size, options.build_date_index);

        for page_idx in 0..total_pages {
            let chunk_start = page_idx * options.chunk_size;
            let chunk_end = (chunk_start + options.chunk_size).min(topic_msgs.len());
            let raw_chunk = if chunk_start < topic_msgs.len() {
                &topic_msgs[chunk_start..chunk_end]
            } else {
                &[]
            };

            let (cont_prev_gid, cont_next_gid) =
                chunk_continuation_gids(&topic_msgs, chunk_start, chunk_end, page_idx, total_pages);

            let render_items =
                group_messages_into_render_items(raw_chunk.to_vec(), cont_prev_gid, cont_next_gid);

            let date_nav_html = if options.build_date_index {
                Some(date_navigator.render_date_jump_menu(
                    current_peer.peer_id,
                    Some(topic.topic_id),
                    false,
                ))
            } else {
                None
            };

            let page_ctx = DialogPageContext {
                current_peer,
                all_peers: render_peers,
                current_topic: Some(topic),
                topics: &current_peer.topics,
                items: &render_items,
                page_index: page_idx,
                total_pages,
                presentation_mode: options.presentation_mode,
                theme: options.theme,
                date_nav_html: date_nav_html.as_deref(),
                available_avatars,
                is_unified_messages_view: false,
                item_topic_ids: None,
            };

            let page_html = render_dialog_page(&page_ctx);
            let page_file_name = ArchiveUrlBuilder::topic_page_file_name(topic.topic_id, page_idx);
            fs::write(peer_chat_dir.join(&page_file_name), page_html)?;
            total_chunks += 1;
        }
    }

    Ok(total_chunks)
}

pub fn render_unified_messages_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    location_map: &ReplyLocationMap,
    options: &ExportOptions,
    available_avatars: &HashSet<PeerId>,
) -> RenderResult<usize> {
    let mut total_chunks = 0;
    let total_u_msgs = all_render_messages.len();
    let total_u_pages = if total_u_msgs == 0 {
        1
    } else {
        total_u_msgs.div_ceil(options.chunk_size)
    };

    let u_date_navigator = build_date_navigator(
        all_render_messages,
        options.chunk_size,
        options.build_date_index,
    );

    for page_idx in 0..total_u_pages {
        let chunk_start = page_idx * options.chunk_size;
        let chunk_end = (chunk_start + options.chunk_size).min(all_render_messages.len());
        let raw_chunk = if chunk_start < all_render_messages.len() {
            &all_render_messages[chunk_start..chunk_end]
        } else {
            &[]
        };

        let (cont_prev_gid, cont_next_gid) = chunk_continuation_gids(
            all_render_messages,
            chunk_start,
            chunk_end,
            page_idx,
            total_u_pages,
        );

        let render_items =
            group_messages_into_render_items(raw_chunk.to_vec(), cont_prev_gid, cont_next_gid);

        let date_nav_html = if options.build_date_index {
            Some(u_date_navigator.render_date_jump_menu(current_peer.peer_id, None, true))
        } else {
            None
        };

        let chunk_topic_ids: Vec<i32> = render_items
            .iter()
            .map(|item| match item {
                RenderItem::Message(m) => location_map
                    .get_location(&m.key)
                    .and_then(|loc| loc.1)
                    .unwrap_or(1),
                RenderItem::Album(a) => a
                    .messages
                    .first()
                    .and_then(|m| location_map.get_location(&m.key).and_then(|loc| loc.1))
                    .unwrap_or(1),
            })
            .collect();

        let page_ctx = DialogPageContext {
            current_peer,
            all_peers: render_peers,
            current_topic: None,
            topics: &current_peer.topics,
            items: &render_items,
            page_index: page_idx,
            total_pages: total_u_pages,
            presentation_mode: options.presentation_mode,
            theme: options.theme,
            date_nav_html: date_nav_html.as_deref(),
            available_avatars,
            is_unified_messages_view: true,
            item_topic_ids: Some(&chunk_topic_ids),
        };

        let page_html = render_dialog_page(&page_ctx);
        let page_file_name = ArchiveUrlBuilder::unified_messages_page_file_name(page_idx);
        fs::write(peer_chat_dir.join(&page_file_name), page_html)?;
        total_chunks += 1;
    }

    Ok(total_chunks)
}

pub fn render_flat_dialog_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    options: &ExportOptions,
    available_avatars: &HashSet<PeerId>,
) -> RenderResult<usize> {
    let mut total_chunks = 0;
    let total_msgs = all_render_messages.len();
    let total_pages = if total_msgs == 0 {
        1
    } else {
        total_msgs.div_ceil(options.chunk_size)
    };

    let date_navigator = build_date_navigator(
        all_render_messages,
        options.chunk_size,
        options.build_date_index,
    );

    for page_idx in 0..total_pages {
        let chunk_start = page_idx * options.chunk_size;
        let chunk_end = (chunk_start + options.chunk_size).min(all_render_messages.len());
        let raw_chunk = if chunk_start < all_render_messages.len() {
            &all_render_messages[chunk_start..chunk_end]
        } else {
            &[]
        };

        let (cont_prev_gid, cont_next_gid) = chunk_continuation_gids(
            all_render_messages,
            chunk_start,
            chunk_end,
            page_idx,
            total_pages,
        );

        let render_items =
            group_messages_into_render_items(raw_chunk.to_vec(), cont_prev_gid, cont_next_gid);

        let date_nav_html = if options.build_date_index {
            Some(date_navigator.render_date_jump_menu(current_peer.peer_id, None, false))
        } else {
            None
        };

        let page_ctx = DialogPageContext {
            current_peer,
            all_peers: render_peers,
            current_topic: None,
            topics: &[],
            items: &render_items,
            page_index: page_idx,
            total_pages,
            presentation_mode: options.presentation_mode,
            theme: options.theme,
            date_nav_html: date_nav_html.as_deref(),
            available_avatars,
            is_unified_messages_view: false,
            item_topic_ids: None,
        };

        let page_html = render_dialog_page(&page_ctx);
        let page_file_name = ArchiveUrlBuilder::page_file_name(page_idx);
        fs::write(peer_chat_dir.join(&page_file_name), page_html)?;
        total_chunks += 1;
    }

    Ok(total_chunks)
}

pub fn write_root_topic_redirect(
    peer_chat_dir: &Path,
    topics: &[RenderTopic],
) -> RenderResult<usize> {
    let default_tid = topics.first().map(|t| t.topic_id).unwrap_or(1);
    let default_file = ArchiveUrlBuilder::topic_page_file_name(default_tid, 0);
    let redirect_html = format!(
        r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={default_file}"><script>window.location.replace("{default_file}");</script></head><body><p>Redirecting to <a href="{default_file}">topics</a>...</p></body></html>"#
    );
    fs::write(peer_chat_dir.join("page_00001.html"), redirect_html)?;
    Ok(1)
}
