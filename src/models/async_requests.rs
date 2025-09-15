use matrix_sdk::{
    OwnedServerName, RoomMemberships,
    media::MediaRequestParameters,
    room::{
        RoomMember,
        edit::EditedContent,
        reply::{EnforceThread, Reply},
    },
    ruma::{
        OwnedEventId, OwnedRoomAliasId, OwnedRoomId, OwnedUserId,
        events::room::message::{
            ReplyWithinThread, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
        },
        matrix_uri::MatrixId,
    },
};
use matrix_sdk_ui::timeline::TimelineEventItemId;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::{events::timeline::PaginationDirection, init::singletons::REQUEST_SENDER};

/// Submits a request to the worker thread to be executed asynchronously.
pub(crate) fn submit_async_request(req: MatrixRequest) {
    REQUEST_SENDER
        .get()
        .unwrap() // this is initialized
        .send(req)
        .expect("BUG: async worker task receiver has died!");
}

/// The set of requests for async work that can be made to the worker thread.
#[allow(clippy::large_enum_variant)]
pub enum MatrixRequest {
    /// Request to paginate the older (or newer) events of a room's timeline.
    PaginateRoomTimeline {
        room_id: OwnedRoomId,
        /// The maximum number of timeline events to fetch in each pagination batch.
        num_events: u16,
        direction: PaginationDirection,
    },
    /// Request to edit the content of an event in the given room's timeline.
    EditMessage {
        room_id: OwnedRoomId,
        timeline_event_item_id: TimelineEventItemId,
        edited_content: EditedContent,
    },
    /// Request to fetch the full details of the given event in the given room's timeline.
    FetchDetailsForEvent {
        room_id: OwnedRoomId,
        event_id: OwnedEventId,
    },
    /// Request to fetch profile information for all members of a room.
    /// This can be *very* slow depending on the number of members in the room.
    SyncRoomMemberList {
        room_id: OwnedRoomId,
    },
    /// Request to join the given room.
    JoinRoom {
        room_id: OwnedRoomId,
    },
    /// Request to leave the given room.
    LeaveRoom {
        room_id: OwnedRoomId,
    },
    /// Request to get the actual list of members in a room.
    /// This returns the list of members that can be displayed in the UI.
    GetRoomMembers {
        room_id: OwnedRoomId,
        memberships: RoomMemberships,
        /// * If `true` (not recommended), only the local cache will be accessed.
        /// * If `false` (recommended), details will be fetched from the server.
        local_only: bool,
    },
    /// Request to fetch profile information for the given user ID.
    GetUserProfile {
        user_id: OwnedUserId,
        /// * If `Some`, the user is known to be a member of a room, so this will
        ///   fetch the user's profile from that room's membership info.
        /// * If `None`, the user's profile info will be fetched from the server
        ///   in a room-agnostic manner, and no room membership info will be returned.
        room_id: Option<OwnedRoomId>,
        /// * If `true` (not recommended), only the local cache will be accessed.
        /// * If `false` (recommended), details will be fetched from the server.
        local_only: bool,
    },
    /// Request to fetch the number of unread messages in the given room.
    GetNumberUnreadMessages {
        room_id: OwnedRoomId,
    },
    /// Request to ignore/block or unignore/unblock a user.
    IgnoreUser {
        /// Whether to ignore (`true`) or unignore (`false`) the user.
        ignore: bool,
        /// The room membership info of the user to (un)ignore.
        room_member: RoomMember,
        /// The room ID of the room where the user is a member,
        /// which is only needed because it isn't present in the `RoomMember` object.
        room_id: OwnedRoomId,
    },
    /// Request to resolve a room alias into a room ID and the servers that know about that room.
    ResolveRoomAlias(OwnedRoomAliasId),
    /// Request to fetch media from the server.
    /// Upon completion of the async media request, the `on_fetched` function
    /// will be invoked with four arguments: the `destination`, the `media_request`,
    /// the result of the media fetch, and the `update_sender`.
    FetchMedia {
        media_request: MediaRequestParameters,
        content_sender: oneshot::Sender<Result<Vec<u8>, matrix_sdk::Error>>,
    },
    /// Request to send a message to the given room.
    SendMessage {
        room_id: OwnedRoomId,
        message: RoomMessageEventContent,
        replied_to: Option<Reply>,
    },
    /// Sends a notice to the given room that the current user is or is not typing.
    ///
    /// This request does not return a response or notify the UI thread, and
    /// furthermore, there is no need to send a follow-up request to stop typing
    /// (though you certainly can do so).
    SendTypingNotice {
        room_id: OwnedRoomId,
        typing: bool,
    },
    /// Subscribe to typing notices for the given room.
    ///
    /// This request does not return a response or notify the UI thread.
    SubscribeToTypingNotices {
        room_id: OwnedRoomId,
        /// Whether to subscribe or unsubscribe from typing notices for this room.
        subscribe: bool,
    },
    /// Subscribe to changes in the read receipts of our own user.
    ///
    /// This request does not return a response or notify the UI thread.
    SubscribeToOwnUserReadReceiptsChanged {
        room_id: OwnedRoomId,
        /// Whether to subscribe or unsubscribe to changes in the read receipts of our own user for this room
        subscribe: bool,
    },
    /// Sends a read receipt for the given event in the given room.
    ReadReceipt {
        room_id: OwnedRoomId,
        event_id: OwnedEventId,
    },
    /// Sends a fully-read receipt for the given event in the given room.
    FullyReadReceipt {
        room_id: OwnedRoomId,
        event_id: OwnedEventId,
    },
    /// Sends a request to obtain the power levels for this room.
    ///
    /// The response is delivered back to the main UI thread via [`TimelineUpdate::UserPowerLevels`].
    GetRoomPowerLevels {
        room_id: OwnedRoomId,
    },
    /// Toggles the given reaction to the given event in the given room.
    ToggleReaction {
        room_id: OwnedRoomId,
        timeline_event_id: TimelineEventItemId,
        reaction: String,
    },
    /// Redacts (deletes) the given event in the given room.
    #[doc(alias("delete"))]
    RedactMessage {
        room_id: OwnedRoomId,
        timeline_event_id: TimelineEventItemId,
        reason: Option<String>,
    },
    /// Sends a request to obtain the room's pill link info for the given Matrix ID.
    ///
    /// The MatrixLinkPillInfo::Loaded variant is sent back to the main UI thread via.
    GetMatrixRoomLinkPillInfo {
        matrix_id: MatrixId,
        via: Vec<OwnedServerName>,
    },
    CreateDMRoom {
        user_id: OwnedUserId,
    },
}
// Deserialize trait is implemented in models/async_requests.rs

impl<'de> Deserialize<'de> for MatrixRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // First deserialize into a generic Value to inspect the structure
        let value = Value::deserialize(deserializer)?;

        // Extract the "event" field to determine the variant
        let event = value
            .get("event")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("event"))?;

        // Extract the "payload" field containing the variant data
        let payload = value
            .get("payload")
            .ok_or_else(|| serde::de::Error::missing_field("payload"))?;

        // Match on the event type and deserialize the appropriate variant
        match event {
            "paginateRoomTimeline" => {
                let data: PaginateRoomTimelinePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::PaginateRoomTimeline {
                    room_id: data.room_id,
                    num_events: data.num_events,
                    direction: data.direction,
                })
            }
            "editMessage" => {
                let data: EditMessagePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::EditMessage {
                    room_id: data.room_id,
                    // We only use remote event_id for now. Transaction (local) ids could be supported in the future
                    timeline_event_item_id: TimelineEventItemId::EventId(
                        data.timeline_event_item_id,
                    ),
                    // We only allow editing messages for now.
                    edited_content: EditedContent::RoomMessage(data.edited_content),
                })
            }
            "fetchDetailsForEvent" => {
                let data: FetchDetailsForEventPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::FetchDetailsForEvent {
                    room_id: data.room_id,
                    event_id: data.event_id,
                })
            }
            // "syncRoomMemberList" => {
            //     let data: SyncRoomMemberListPayload =
            //         serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
            //     Ok(MatrixRequest::SyncRoomMemberList {
            //         room_id: data.room_id,
            //     })
            // }
            "joinRoom" => {
                let data: JoinRoomPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::JoinRoom {
                    room_id: data.room_id,
                })
            }
            "leaveRoom" => {
                let data: LeaveRoomPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::LeaveRoom {
                    room_id: data.room_id,
                })
            }
            // "getRoomMembers" => {
            //     let data: GetRoomMembersPayload =
            //         serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
            //     Ok(MatrixRequest::GetRoomMembers {
            //         room_id: data.room_id,
            //         memberships: data.memberships,
            //         local_only: data.local_only,
            //     })
            // }
            "getUserProfile" => {
                let data: GetUserProfilePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::GetUserProfile {
                    user_id: data.user_id,
                    room_id: data.room_id,
                    local_only: data.local_only,
                })
            }
            "getNumberUnreadMessages" => {
                let data: GetNumberUnreadMessagesPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::GetNumberUnreadMessages {
                    room_id: data.room_id,
                })
            }
            // "ignoreUser" => {
            //     let data: IgnoreUserPayload =
            //         serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
            //     Ok(MatrixRequest::IgnoreUser {
            //         ignore: data.ignore,
            //         room_member: data.room_member,
            //         room_id: data.room_id,
            //     })
            // }
            "resolveRoomAlias" => {
                let alias =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::ResolveRoomAlias(alias))
            }
            "sendMessage" => {
                let data: SendMessagePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                let reply_option = match data.reply_to_event_id {
                    Some(id) => Some(Reply {
                        event_id: id,
                        enforce_thread: EnforceThread::Threaded(ReplyWithinThread::Yes),
                    }),
                    None => None,
                };
                Ok(MatrixRequest::SendMessage {
                    room_id: data.room_id,
                    message: data.message,
                    replied_to: reply_option,
                })
            }
            "sendTypingNotice" => {
                let data: SendTypingNoticePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::SendTypingNotice {
                    room_id: data.room_id,
                    typing: data.typing,
                })
            }
            "subscribeToTypingNotices" => {
                let data: SubscribeToTypingNoticesPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::SubscribeToTypingNotices {
                    room_id: data.room_id,
                    subscribe: data.subscribe,
                })
            }
            "subscribeToOwnUserReadReceiptsChanged" => {
                let data: SubscribeToOwnUserReadReceiptsChangedPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::SubscribeToOwnUserReadReceiptsChanged {
                    room_id: data.room_id,
                    subscribe: data.subscribe,
                })
            }
            "readReceipt" => {
                let data: ReadReceiptPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::ReadReceipt {
                    room_id: data.room_id,
                    event_id: data.event_id,
                })
            }
            "fullyReadReceipt" => {
                let data: FullyReadReceiptPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::FullyReadReceipt {
                    room_id: data.room_id,
                    event_id: data.event_id,
                })
            }
            "getRoomPowerLevels" => {
                let data: GetRoomPowerLevelsPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::GetRoomPowerLevels {
                    room_id: data.room_id,
                })
            }
            "toggleReaction" => {
                let data: ToggleReactionPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::ToggleReaction {
                    room_id: data.room_id,
                    timeline_event_id: TimelineEventItemId::EventId(
                        OwnedEventId::try_from(data.timeline_event_id)
                            .expect("Frontend sent incorrect event id"),
                    ), // We only use eventId, not transactions.
                    reaction: data.reaction,
                })
            }
            "redactMessage" => {
                let data: RedactMessagePayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::RedactMessage {
                    room_id: data.room_id,
                    timeline_event_id: TimelineEventItemId::EventId(data.timeline_event_id),
                    reason: data.reason,
                })
            }
            // "getMatrixRoomLinkPillInfo" => {
            //     let data: GetMatrixRoomLinkPillInfoPayload =
            //         serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
            //     Ok(MatrixRequest::GetMatrixRoomLinkPillInfo {
            //         matrix_id: data.matrix_id,
            //         via: data.via,
            //     })
            // }
            "createDMRoom" => {
                let data: CreateDMRoomPayload =
                    serde_json::from_value(payload.clone()).map_err(serde::de::Error::custom)?;
                Ok(MatrixRequest::CreateDMRoom {
                    user_id: data.user_id,
                })
            }
            _ => Err(serde::de::Error::unknown_variant(
                event,
                &[
                    "paginateRoomTimeline",
                    "editMessage",
                    "fetchDetailsForEvent",
                    // "syncRoomMemberList",
                    "joinRoom",
                    "leaveRoom",
                    // "getRoomMembers",
                    "getUserProfile",
                    "getNumberUnreadMessages",
                    // "ignoreUser",
                    "resolveRoomAlias",
                    "sendMessage",
                    "sendTypingNotice",
                    "subscribeToTypingNotices",
                    "subscribeToOwnUserReadReceiptsChanged",
                    "readReceipt",
                    "fullyReadReceipt",
                    "getRoomPowerLevels",
                    "toggleReaction",
                    "redactMessage",
                    // "getMatrixRoomLinkPillInfo",
                    "createDMRoom",
                ],
            )),
        }
    }
}

// Helper structs for deserializing payloads
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaginateRoomTimelinePayload {
    room_id: OwnedRoomId,
    num_events: u16,
    direction: PaginationDirection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditMessagePayload {
    room_id: OwnedRoomId,
    timeline_event_item_id: OwnedEventId,
    edited_content: RoomMessageEventContentWithoutRelation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchDetailsForEventPayload {
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
}

// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// struct SyncRoomMemberListPayload {
//     room_id: OwnedRoomId,
// }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinRoomPayload {
    room_id: OwnedRoomId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaveRoomPayload {
    room_id: OwnedRoomId,
}

// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// struct GetRoomMembersPayload {
//     room_id: OwnedRoomId,
//     memberships: RoomMemberships,
//     local_only: bool,
// }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetUserProfilePayload {
    user_id: OwnedUserId,
    room_id: Option<OwnedRoomId>,
    local_only: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetNumberUnreadMessagesPayload {
    room_id: OwnedRoomId,
}

// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// struct IgnoreUserPayload {
//     ignore: bool,
//     room_member: RoomMember,
//     room_id: OwnedRoomId,
// }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessagePayload {
    room_id: OwnedRoomId,
    message: RoomMessageEventContent,
    reply_to_event_id: Option<OwnedEventId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendTypingNoticePayload {
    room_id: OwnedRoomId,
    typing: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeToTypingNoticesPayload {
    room_id: OwnedRoomId,
    subscribe: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeToOwnUserReadReceiptsChangedPayload {
    room_id: OwnedRoomId,
    subscribe: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadReceiptPayload {
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FullyReadReceiptPayload {
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetRoomPowerLevelsPayload {
    room_id: OwnedRoomId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToggleReactionPayload {
    room_id: OwnedRoomId,
    timeline_event_id: String,
    reaction: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RedactMessagePayload {
    room_id: OwnedRoomId,
    timeline_event_id: OwnedEventId,
    reason: Option<String>,
}

// #[derive(Deserialize)]
// #[serde(rename_all = "camelCase")]
// struct GetMatrixRoomLinkPillInfoPayload {
//     matrix_id: MatrixId,
//     via: Vec<OwnedServerName>,
// }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateDMRoomPayload {
    user_id: OwnedUserId,
}
