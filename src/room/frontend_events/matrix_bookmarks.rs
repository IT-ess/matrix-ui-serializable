use matrix_sdk::{
    Room,
    deserialized_responses::TimelineEvent,
    ruma::{OwnedMxcUri, OwnedRoomId},
};
use matrix_sdk_ui::timeline::{TimelineEventItemId, TimelineItemContent};
use serde::Serialize;

use crate::{
    FrontendTimelineItem,
    room::frontend_events::{
        events_dto::{MessageAbilities, map_timeline_event_item_content},
        timeline_item_id::FrontendTimelineEventItemId,
    },
    user::user_profile::with_user_profile,
};

/// High-level struct that wraps all information about a bookmark
/// for frontend usage
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixBookmarkItem {
    /// The bookmarked event that has been fetched and mapped to a frontend
    /// timeline item.
    /// These are only MsgLike events.
    item: FrontendTimelineItem,
    /// The room_id where this event is in
    room_id: OwnedRoomId,
    /// The room_display_name of the room this event is in
    room_name: Option<String>,
    /// The avatar of the room this event is in
    room_avatar: Option<OwnedMxcUri>,
    /// The avatar_uri of this bookmark's sender
    sender_avatar: Option<OwnedMxcUri>,
}

pub async fn to_matrix_bookmark_item(
    unique_id: String,
    room: &Room,
    event: TimelineEvent,
) -> Option<MatrixBookmarkItem> {
    let event_id = event.event_id()?.to_owned();
    let sender_id = event.sender()?;
    let timestamp = event.timestamp.map(|t| t.0);
    let sender_info = with_user_profile(
        sender_id.clone(),
        Some(&room.room_id().to_owned()),
        true,
        |profile, _| {
            (
                profile.displayable_name().to_owned(),
                profile.avatar.clone(),
            )
        },
    );
    let item_content = TimelineItemContent::from_event(room, event).await?;
    let item = map_timeline_event_item_content(
        &item_content,
        unique_id,
        FrontendTimelineEventItemId(TimelineEventItemId::EventId(event_id.clone())),
        false, // TODO: compute is_own ?
        false,
        timestamp,
        sender_info.clone().map(|i| i.0),
        sender_id.to_string(),
        MessageAbilities::all(),
        Some(event_id),
    )?;

    Some(MatrixBookmarkItem {
        item,
        room_id: room.room_id().to_owned(),
        room_name: room.cached_display_name().map(|name| name.to_string()),
        room_avatar: room.avatar_url(),
        sender_avatar: sender_info.and_then(|i| i.1),
    })
}
