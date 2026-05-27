use matrix_sdk::{
    RoomHero, RoomState,
    room_preview::RoomPreview,
    ruma::{
        OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId,
        room::{JoinRuleSummary, RoomType},
    },
};
use serde::Serialize;

/// The preview of a room, be it invited/joined/left, or not.
#[derive(Debug, Clone, Serialize)]
pub struct SerializableRoomPreview {
    /// The actual room id for this room.
    ///
    /// Remember the room preview can be fetched from a room alias id, so we
    /// might not know ahead of time what the room id is.
    pub room_id: OwnedRoomId,

    /// The canonical alias for the room.
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// The room's name, if set.
    pub name: Option<String>,

    /// The room's topic, if set.
    pub topic: Option<String>,

    /// The MXC URI to the room's avatar, if set.
    pub avatar_url: Option<OwnedMxcUri>,

    /// The number of joined members.
    pub num_joined_members: u64,

    /// The number of active members, if known (joined + invited).
    pub num_active_members: Option<u64>,

    /// The room type (space, custom) or nothing, if it's a regular room.
    pub room_type: Option<RoomType>,

    /// What's the join rule for this room?
    pub join_rule: Option<JoinRuleSummary>,

    /// Is the room world-readable (i.e. is its history_visibility set to
    /// world_readable)?
    pub is_world_readable: Option<bool>,

    /// Has the current user been invited/joined/left this room?
    ///
    /// Set to `None` if the room is unknown to the user.
    pub state: Option<RoomState>,

    /// The `m.room.direct` state of the room, if known.
    pub is_direct: Option<bool>,

    /// Room heroes.
    pub heroes: Option<Vec<RoomHero>>,
}

impl From<RoomPreview> for SerializableRoomPreview {
    fn from(value: RoomPreview) -> Self {
        let RoomPreview {
            room_id,
            avatar_url,
            canonical_alias,
            heroes,
            is_direct,
            is_world_readable,
            join_rule,
            name,
            num_active_members,
            num_joined_members,
            room_type,
            state,
            topic,
        } = value;
        Self {
            room_id,
            canonical_alias,
            name,
            topic,
            avatar_url,
            num_joined_members,
            num_active_members,
            room_type,
            join_rule,
            is_world_readable,
            state,
            is_direct,
            heroes,
        }
    }
}
