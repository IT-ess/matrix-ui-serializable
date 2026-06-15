use matrix_sdk::{
    bookmarks::IndexedBookmark,
    ruma::{MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedUserId},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Payload to edit current user's information.
/// Only the Some(...) fields are updated, None are ignored.
pub struct EditUserInformationPayload {
    pub new_display_name: Option<String>,
    pub new_avatar_uri: Option<OwnedMxcUri>,
    pub new_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Payload to edit current user's information.
/// Only the Some(...) fields are updated, None are ignored.
pub struct EditRoomInformationPayload {
    pub room_id: OwnedRoomId,
    pub new_display_name: Option<String>,
    pub new_avatar_uri: Option<OwnedMxcUri>,
    pub topic: Option<String>,
}

/// Representation of a bookmark as it is stored
/// in the index. + the serialize trait
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendIndexedBookmark {
    /// Event id of the current "version" of the bookmarked
    /// message (latest event of the `m.replace` relation chain)
    pub event_id: OwnedEventId,
    /// "Root" event id of the bookmarked message (first event of
    /// the `m.replace` relation chain)
    pub original_event_id: OwnedEventId,
    /// Event id of the `m.bookmark` event that points to the
    /// bookmarked event and triggered its indexation.
    pub pointer_event_id: OwnedEventId,
    /// Body of the bookmarked message. Maybe an empty string if
    /// the event does not have a string representation.
    pub body: String,
    /// When the bookmarked event has been sent
    pub original_server_ts: MilliSecondsSinceUnixEpoch,
    /// Sender of the bookmarked event
    pub sender: OwnedUserId,
    /// Room in which the bookmarked event lives
    pub room_id: OwnedRoomId,
    /// Search score
    pub score: f32,
}

impl From<IndexedBookmark> for FrontendIndexedBookmark {
    fn from(value: IndexedBookmark) -> Self {
        Self {
            event_id: value.event_id,
            original_event_id: value.original_event_id,
            pointer_event_id: value.pointer_event_id,
            body: value.body,
            original_server_ts: value.original_server_ts,
            sender: value.sender,
            room_id: value.room_id,
            score: value.score,
        }
    }
}
