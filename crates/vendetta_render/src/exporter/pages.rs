use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
};

use vendetta_model::{MessageRecord, PeerId};

use crate::{
    error::RenderResult,
    layout::dialog::{render_dialog_page, DialogPageContext},
    message::{edits::days_to_ymd, group_messages_into_render_items},
    model::{DateStructure, ExportOptions, RenderItem, RenderMessage, RenderPeer, RenderTopic, SplitBy},
    navigation::DateNavigator,
    reply::ReplyLocationMap,
    url_builder::ArchiveUrlBuilder,
};

pub fn group_records_by_day(
    messages: &[MessageRecord],
) -> Vec<((i32, u32, u32), Vec<MessageRecord>)> {
    messages
        .iter()
        .fold(BTreeMap::new(), |mut acc, m| {
            acc.entry(days_to_ymd(m.date / 86400))
                .or_insert_with(Vec::new)
                .push(m.clone());
            acc
        })
        .into_iter()
        .collect()
}

pub fn group_messages_by_day(
    messages: &[RenderMessage],
) -> Vec<((i32, u32, u32), Vec<RenderMessage>)> {
    messages
        .iter()
        .fold(BTreeMap::new(), |mut acc, m| {
            acc.entry(days_to_ymd(m.date / 86400))
                .or_insert_with(Vec::new)
                .push(m.clone());
            acc
        })
        .into_iter()
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn render_single_day_dialog_page(
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    current_topic: Option<&RenderTopic>,
    topics: &[RenderTopic],
    day_msgs: &[RenderMessage],
    day_idx: usize,
    total_days: usize,
    file_rel: &str,
    day_file_names: &[String],
    options: &ExportOptions,
    date_structure: DateStructure,
    available_avatars: &HashSet<PeerId>,
    date_navigator: Option<&DateNavigator>,
    chat_dirs: Option<&HashMap<PeerId, String>>,
) -> String {
    let (y, m, d) = day_msgs
        .first()
        .map(|msg| days_to_ymd(msg.date / 86400))
        .unwrap_or((1970, 1, 1));
    let render_items = group_messages_into_render_items(day_msgs.to_vec(), None, None);
    let custom_page_indicator =
        format!("{y:04}-{m:02}-{d:02} (Day {} of {})", day_idx + 1, total_days);
    let custom_prev_url = day_idx
        .checked_sub(1)
        .and_then(|prev_idx| day_file_names.get(prev_idx))
        .map(|prev_file| ArchiveUrlBuilder::relative_day_to_day(file_rel, prev_file));
    let custom_next_url = day_file_names
        .get(day_idx + 1)
        .map(|next_file| ArchiveUrlBuilder::relative_day_to_day(file_rel, next_file));

    let in_topic = current_topic.is_some();
    let chat_depth = ArchiveUrlBuilder::day_page_depth(in_topic, date_structure);

    let date_nav_html = if options.build_date_index {
        date_navigator.map(|nav| nav.render_date_jump_menu_for_page(Some(file_rel)))
    } else {
        None
    };

    let page_ctx = DialogPageContext {
        current_peer,
        all_peers: render_peers,
        current_topic,
        topics,
        items: &render_items,
        page_index: day_idx,
        total_pages: total_days,
        presentation_mode: options.presentation_mode,
        theme: options.theme,
        date_nav_html: date_nav_html.as_deref(),
        available_avatars,
        is_unified_messages_view: false,
        item_topic_ids: None,
        chat_depth,
        custom_prev_url,
        custom_next_url,
        custom_page_indicator: Some(custom_page_indicator),
        chat_dirs,
    };

    render_dialog_page(&page_ctx)
}

fn build_date_navigator(msgs: &[RenderMessage], chunk_size: usize, enabled: bool) -> DateNavigator {
    if !enabled {
        return DateNavigator::new();
    }
    msgs.iter()
        .enumerate()
        .fold(DateNavigator::new(), |mut nav, (msg_idx, m)| {
            nav.record_message_date(m.date, msg_idx / chunk_size);
            nav
        })
}

fn chunk_continuation_gids(
    msgs: &[RenderMessage],
    chunk_start: usize,
    chunk_end: usize,
    page_idx: usize,
    total_pages: usize,
) -> (Option<i64>, Option<i64>) {
    let cont_prev_gid = (page_idx > 0 && chunk_start > 0)
        .then(|| {
            let prev_last = msgs.get(chunk_start.checked_sub(1)?)?;
            let curr_first = msgs.get(chunk_start)?;
            (prev_last.grouped_id.is_some() && prev_last.grouped_id == curr_first.grouped_id)
                .then_some(curr_first.grouped_id)
                .flatten()
        })
        .flatten();

    let cont_next_gid = (page_idx + 1 < total_pages && chunk_end < msgs.len())
        .then(|| {
            let curr_last = msgs.get(chunk_end.checked_sub(1)?)?;
            let next_first = msgs.get(chunk_end)?;
            (curr_last.grouped_id.is_some() && curr_last.grouped_id == next_first.grouped_id)
                .then_some(curr_last.grouped_id)
                .flatten()
        })
        .flatten();

    (cont_prev_gid, cont_next_gid)
}

#[allow(clippy::too_many_arguments)]
pub fn render_topic_scoped_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    location_map: &ReplyLocationMap,
    options: &ExportOptions,
    split_by: SplitBy,
    date_structure: DateStructure,
    available_avatars: &HashSet<PeerId>,
    chat_dirs: Option<&HashMap<PeerId, String>>,
) -> RenderResult<Vec<String>> {
    let mut created_pages = Vec::new();

    for topic in &current_peer.topics {
        let topic_dir = peer_chat_dir.join("topics").join(topic.topic_id.to_string());
        fs::create_dir_all(&topic_dir)?;

        let topic_msgs: Vec<RenderMessage> = all_render_messages
            .iter()
            .filter(|m| {
                location_map
                    .get_location(&m.key)
                    .and_then(|loc| loc.1)
                    .unwrap_or(1)
                    == topic.topic_id
            })
            .cloned()
            .collect();

        if split_by == SplitBy::Day && !topic_msgs.is_empty() {
            let day_groups = group_messages_by_day(&topic_msgs);
            let total_days = day_groups.len();
            let day_file_names: Vec<String> = day_groups
                .iter()
                .map(|((y, m, d), _)| {
                    ArchiveUrlBuilder::day_page_file_name(*y, *m, *d, date_structure)
                })
                .collect();

            let mut date_navigator = DateNavigator::new();
            if options.build_date_index {
                for (((_y, _m, _d), msgs), file_name) in day_groups.iter().zip(&day_file_names) {
                    if let Some(first_msg) = msgs.first() {
                        date_navigator.record_message_target(first_msg.date, file_name.clone());
                    }
                }
            }

            for (day_idx, (((_y, _m, _d), day_msgs), file_rel)) in
                day_groups.iter().zip(&day_file_names).enumerate()
            {
                let page_html = render_single_day_dialog_page(
                    current_peer,
                    render_peers,
                    Some(topic),
                    &current_peer.topics,
                    day_msgs,
                    day_idx,
                    total_days,
                    file_rel,
                    &day_file_names,
                    options,
                    date_structure,
                    available_avatars,
                    Some(&date_navigator),
                    chat_dirs,
                );

                let out_path = topic_dir.join(file_rel);
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(out_path, page_html)?;
                created_pages.push(format!("topics/{}/{}", topic.topic_id, file_rel));
            }

            if let Some(earliest_day_file) = day_file_names.first() {
                let index_redirect = format!(
                    r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={earliest_day_file}"><script>window.location.replace("{earliest_day_file}");</script></head><body><p>Redirecting to <a href="{earliest_day_file}">topic</a>...</p></body></html>"#
                );
                fs::write(topic_dir.join("index.html"), index_redirect)?;
                created_pages.push(format!("topics/{}/index.html", topic.topic_id));
            }
            continue;
        }

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

            let (cont_prev_gid, cont_next_gid) = chunk_continuation_gids(
                &topic_msgs,
                chunk_start,
                chunk_end,
                page_idx,
                total_pages,
            );

            let render_items = group_messages_into_render_items(
                raw_chunk.to_vec(),
                cont_prev_gid,
                cont_next_gid,
            );

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
                chat_depth: 2,
                custom_prev_url: None,
                custom_next_url: None,
                custom_page_indicator: None,
                chat_dirs,
            };

            let page_html = render_dialog_page(&page_ctx);
            let page_file_name = ArchiveUrlBuilder::page_file_name(page_idx);
            fs::write(topic_dir.join(&page_file_name), page_html)?;
            created_pages.push(format!("topics/{}/{}", topic.topic_id, page_file_name));
        }
    }

    Ok(created_pages)
}

#[allow(clippy::too_many_arguments)]
pub fn render_unified_messages_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    location_map: &ReplyLocationMap,
    options: &ExportOptions,
    available_avatars: &HashSet<PeerId>,
    chat_dirs: Option<&HashMap<PeerId, String>>,
) -> RenderResult<Vec<String>> {
    let mut created_pages = Vec::new();
    let messages_dir = peer_chat_dir.join("topics").join("messages");
    fs::create_dir_all(&messages_dir)?;

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
            chat_depth: 2,
            custom_prev_url: None,
            custom_next_url: None,
            custom_page_indicator: None,
            chat_dirs,
        };

        let page_html = render_dialog_page(&page_ctx);
        let page_file_name = ArchiveUrlBuilder::page_file_name(page_idx);
        fs::write(messages_dir.join(&page_file_name), page_html)?;
        created_pages.push(format!("topics/messages/{page_file_name}"));
    }

    Ok(created_pages)
}

#[allow(clippy::too_many_arguments)]
pub fn render_flat_dialog_pages(
    peer_chat_dir: &Path,
    current_peer: &RenderPeer,
    render_peers: &[RenderPeer],
    all_render_messages: &[RenderMessage],
    options: &ExportOptions,
    split_by: SplitBy,
    date_structure: DateStructure,
    available_avatars: &HashSet<PeerId>,
    chat_dirs: Option<&HashMap<PeerId, String>>,
) -> RenderResult<Vec<String>> {
    let mut created_pages = Vec::new();

    if split_by == SplitBy::Day && !all_render_messages.is_empty() {
        let day_groups = group_messages_by_day(all_render_messages);
        let total_days = day_groups.len();
        let day_file_names: Vec<String> = day_groups
            .iter()
            .map(|((y, m, d), _)| {
                ArchiveUrlBuilder::day_page_file_name(*y, *m, *d, date_structure)
            })
            .collect();

        let mut date_navigator = DateNavigator::new();
        if options.build_date_index {
            for (((_y, _m, _d), msgs), file_name) in day_groups.iter().zip(&day_file_names) {
                if let Some(first_msg) = msgs.first() {
                    date_navigator.record_message_target(first_msg.date, file_name.clone());
                }
            }
        }

        for (day_idx, (((_y, _m, _d), day_msgs), file_rel)) in
            day_groups.iter().zip(&day_file_names).enumerate()
        {
            let page_html = render_single_day_dialog_page(
                current_peer,
                render_peers,
                None,
                &[],
                day_msgs,
                day_idx,
                total_days,
                file_rel,
                &day_file_names,
                options,
                date_structure,
                available_avatars,
                Some(&date_navigator),
                chat_dirs,
            );

            let out_path = peer_chat_dir.join(file_rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(out_path, page_html)?;
            created_pages.push(file_rel.clone());
        }

        if let Some(earliest_day_file) = day_file_names.first() {
            let index_redirect = format!(
                r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={earliest_day_file}"><script>window.location.replace("{earliest_day_file}");</script></head><body><p>Redirecting to <a href="{earliest_day_file}">chat</a>...</p></body></html>"#
            );
            fs::write(peer_chat_dir.join("index.html"), index_redirect)?;
            created_pages.push("index.html".to_string());
        }

        return Ok(created_pages);
    }

    let total_msgs = all_render_messages.len();
    let total_pages = if total_msgs == 0 {
        1
    } else {
        total_msgs.div_ceil(options.chunk_size)
    };

    let date_navigator =
        build_date_navigator(all_render_messages, options.chunk_size, options.build_date_index);

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
            chat_depth: 0,
            custom_prev_url: None,
            custom_next_url: None,
            custom_page_indicator: None,
            chat_dirs,
        };

        let page_html = render_dialog_page(&page_ctx);
        let page_file_name = ArchiveUrlBuilder::page_file_name(page_idx);
        fs::write(peer_chat_dir.join(&page_file_name), page_html)?;
        created_pages.push(page_file_name);
    }

    let index_redirect = r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url=page_00001.html"><script>window.location.replace("page_00001.html");</script></head><body><p>Redirecting to <a href="page_00001.html">chat</a>...</p></body></html>"#;
    fs::write(peer_chat_dir.join("index.html"), index_redirect)?;
    created_pages.push("index.html".to_string());

    Ok(created_pages)
}

pub fn write_root_topic_redirect(
    peer_chat_dir: &Path,
    topics: &[RenderTopic],
    _options: &ExportOptions,
    split_by: SplitBy,
) -> RenderResult<Vec<String>> {
    let default_tid = topics.first().map(|t| t.topic_id).unwrap_or(1);
    let default_url = if split_by == SplitBy::Day {
        format!("topics/{default_tid}/index.html")
    } else {
        format!("topics/{default_tid}/page_00001.html")
    };
    let redirect_html = format!(
        r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={default_url}"><script>window.location.replace("{default_url}");</script></head><body><p>Redirecting to <a href="{default_url}">topics</a>...</p></body></html>"#
    );
    fs::write(peer_chat_dir.join("index.html"), &redirect_html)?;
    fs::write(peer_chat_dir.join("page_00001.html"), &redirect_html)?;
    Ok(vec!["index.html".to_string(), "page_00001.html".to_string()])
}
