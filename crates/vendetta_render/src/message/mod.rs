pub mod bubble;
pub mod deletes;
pub mod edits;
pub mod forwards;
pub mod reactions;
pub mod service;

pub use bubble::{GroupingContext, render_album_bubble, render_chat_item, render_message_bubble};
pub use deletes::render_state_indicator;
pub use edits::{chrono_like_format, days_to_ymd, render_edit_history};
pub use forwards::render_forward_header;
pub use reactions::render_message_reactions;
pub use service::{format_short_time, render_service_message};

use crate::model::{RenderAlbum, RenderItem, RenderMessage};

const GROUP_TIME_WINDOW_SECS: i64 = 600;

pub fn group_messages_into_render_items(
    messages: Vec<RenderMessage>,
    continuation_prev_gid: Option<i64>,
    continuation_next_gid: Option<i64>,
) -> Vec<RenderItem> {
    let total = messages.len();
    if total == 0 {
        return Vec::new();
    }

    let mut items = Vec::new();
    let mut msgs_iter = messages.into_iter().enumerate().peekable();

    while let Some((i, msg)) = msgs_iter.next() {
        if let Some(gid) = msg.grouped_id {
            let mut album_msgs = vec![msg];
            let mut last_idx = i + 1;

            while let Some((_, next_msg)) = msgs_iter.next_if(|(_, m)| m.grouped_id == Some(gid)) {
                album_msgs.push(next_msg);
                last_idx += 1;
            }

            let cont_prev = i == 0 && continuation_prev_gid == Some(gid);
            let cont_next = last_idx == total && continuation_next_gid == Some(gid);

            if album_msgs.len() == 1
                && !cont_prev
                && !cont_next
                && album_msgs[0].media_items.len() <= 1
            {
                items.push(RenderItem::Message(Box::new(album_msgs.pop().unwrap())));
            } else {
                let album_media = album_msgs
                    .iter()
                    .flat_map(|m| m.media_items.clone())
                    .collect();

                items.push(RenderItem::Album(RenderAlbum {
                    grouped_id: gid,
                    messages: album_msgs,
                    media_items: album_media,
                    continuation_prev: cont_prev,
                    continuation_next: cont_next,
                }));
            }
        } else {
            items.push(RenderItem::Message(Box::new(msg)));
        }
    }

    items
}

fn item_primary_message(item: &RenderItem) -> Option<&RenderMessage> {
    match item {
        RenderItem::Message(m) => Some(m.as_ref()),
        RenderItem::Album(a) => a.messages.first(),
    }
}

fn is_same_cluster(a: &RenderMessage, b: &RenderMessage) -> bool {
    !a.is_service
        && !b.is_service
        && a.sender_id == b.sender_id
        && a.is_outgoing == b.is_outgoing
        && (a.date - b.date).abs() <= GROUP_TIME_WINDOW_SECS
}

pub fn compute_item_grouping_contexts(
    items: &[RenderItem],
    is_group_chat: bool,
) -> Vec<GroupingContext> {
    if items.is_empty() {
        return Vec::new();
    }

    let mut contexts = Vec::with_capacity(items.len());

    for i in 0..items.len() {
        let Some(current_msg) = item_primary_message(&items[i]) else {
            contexts.push(GroupingContext::default());
            continue;
        };

        if current_msg.is_service {
            contexts.push(GroupingContext::default());
            continue;
        }

        let is_prev_same = i > 0
            && item_primary_message(&items[i - 1])
                .is_some_and(|prev| is_same_cluster(prev, current_msg));

        let is_next_same = i + 1 < items.len()
            && item_primary_message(&items[i + 1])
                .is_some_and(|next| is_same_cluster(current_msg, next));

        let is_first = !is_prev_same;
        let is_last = !is_next_same;
        let is_channel = current_msg.is_channel_post;

        contexts.push(GroupingContext {
            is_first_in_group: is_first,
            is_last_in_group: is_last,
            show_sender: is_first && is_group_chat && !current_msg.is_outgoing && !is_channel,
            show_avatar: is_first && is_group_chat && !current_msg.is_outgoing && !is_channel,
        });
    }

    contexts
}

pub fn compute_grouping_contexts(
    messages: &[RenderMessage],
    is_group_chat: bool,
) -> Vec<GroupingContext> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut contexts = Vec::with_capacity(messages.len());

    for i in 0..messages.len() {
        let current = &messages[i];

        if current.is_service {
            contexts.push(GroupingContext::default());
            continue;
        }

        let is_prev_same = i > 0 && is_same_cluster(&messages[i - 1], current);
        let is_next_same = i + 1 < messages.len() && is_same_cluster(current, &messages[i + 1]);

        let is_first = !is_prev_same;
        let is_last = !is_next_same;
        let is_channel = current.is_channel_post;

        contexts.push(GroupingContext {
            is_first_in_group: is_first,
            is_last_in_group: is_last,
            show_sender: is_first && is_group_chat && !current.is_outgoing && !is_channel,
            show_avatar: is_first && is_group_chat && !current.is_outgoing && !is_channel,
        });
    }

    contexts
}
