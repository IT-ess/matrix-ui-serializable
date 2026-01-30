use bitflags::bitflags;
use std::sync::Arc;
use ts_rs::TS;

use matrix_sdk::ruma::{
    OwnedEventId, OwnedRoomId, UInt, event_id, events::room::message::MessageType,
};
use matrix_sdk_ui::timeline::{
    EventTimelineItem, MsgLikeKind, TimelineEventItemId, TimelineItem, TimelineItemContent,
    TimelineItemKind, VirtualTimelineItem,
};
use serde::{Serialize, Serializer};

use crate::{
    room::frontend_events::{
        msg_like::FrontendStickerEventContent,
        state_event::{
            FrontendAnyOtherFullStateEventContent, FrontendMemberProfileChange,
            FrontendRoomMembershipChange, FrontendStateEvent,
        },
        thread_summary::get_frontend_thread_summary,
        timeline_item_id::FrontendTimelineEventItemId,
    },
    user::user_power_level::UserPowerLevels,
    utils::get_or_fetch_event_sender,
};

use super::{
    msg_like::{FrontendMsgLikeContent, FrontendMsgLikeKind},
    virtual_event::FrontendVirtualTimelineItem,
};

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FrontendTimelineItem {
    unique_id: String,
    event_id: Option<OwnedEventId>,
    #[serde(flatten)]
    #[ts(skip)] // Manually typed because flatten isn't compatible
    timeline_item_id: FrontendTimelineEventItemId,
    #[serde(flatten)]
    data: FrontendTimelineItemData,
    #[ts(type = "number | null")]
    timestamp: Option<UInt>, // We keep the timestamp at root to sort events
    is_own: bool,
    is_local: bool,
    #[ts(skip)] // This one stays manually typed
    abilities: MessageAbilities,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "data"
)]
pub enum FrontendTimelineItemData {
    MsgLike(FrontendMsgLikeContent),
    Virtual(FrontendVirtualTimelineItem),
    #[ts(skip)] // This one stays manually typed
    StateChange(FrontendStateEvent),
    Error(FrontendTimelineErrorItem),
    Call,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTimelineErrorItem {
    error: String,
}

pub fn to_frontend_timeline_item(
    item: &Arc<TimelineItem>,
    room_id: Option<&OwnedRoomId>,
    user_power_levels: &UserPowerLevels,
) -> FrontendTimelineItem {
    let unique_id = item.unique_id().0.clone();
    match item.kind() {
        TimelineItemKind::Event(event_tl_item) => {
            let timeline_item_id: FrontendTimelineEventItemId = event_tl_item.identifier().into();
            let is_own = event_tl_item.is_own();
            let is_local = event_tl_item.is_local_echo();
            let timestamp = Some(event_tl_item.timestamp().get());
            let sender = Some(get_or_fetch_event_sender(event_tl_item, room_id));
            let sender_id = event_tl_item.sender().to_string();
            let abilities =
                MessageAbilities::from_user_power_and_event(user_power_levels, event_tl_item);
            let event_id = event_tl_item.event_id().map(|id| id.to_owned());
            map_timeline_event_item_content(
                event_tl_item.content(),
                unique_id,
                timeline_item_id,
                is_own,
                is_local,
                timestamp,
                sender,
                sender_id,
                abilities,
                event_id,
            )
        }
        TimelineItemKind::Virtual(event) => match event {
            VirtualTimelineItem::DateDivider(timestamp) => FrontendTimelineItem {
                unique_id,
                event_id: None,
                timeline_item_id: TimelineEventItemId::EventId(
                    event_id!("$no_ids_for_virtual").to_owned(),
                )
                .into(),
                data: FrontendTimelineItemData::Virtual(FrontendVirtualTimelineItem::DateDivider),
                is_local: true,
                is_own: true,
                timestamp: Some(timestamp.0),
                abilities: MessageAbilities::empty(),
            },
            VirtualTimelineItem::ReadMarker => FrontendTimelineItem {
                unique_id,
                event_id: None,
                timeline_item_id: TimelineEventItemId::EventId(
                    event_id!("$no_ids_for_virtual").to_owned(),
                )
                .into(),
                data: FrontendTimelineItemData::Virtual(FrontendVirtualTimelineItem::ReadMarker),
                is_local: true,
                is_own: true,
                timestamp: None,
                abilities: MessageAbilities::empty(),
            },
            VirtualTimelineItem::TimelineStart => FrontendTimelineItem {
                unique_id,
                event_id: None,
                timeline_item_id: TimelineEventItemId::EventId(
                    event_id!("$no_ids_for_virtual").to_owned(),
                )
                .into(),
                data: FrontendTimelineItemData::Virtual(FrontendVirtualTimelineItem::TimelineStart),
                is_local: true,
                is_own: true,
                timestamp: None,
                abilities: MessageAbilities::empty(),
            },
        },
    }
}

fn map_msg_event_content(content: MessageType) -> FrontendMsgLikeKind {
    match content {
        MessageType::Audio(c) => FrontendMsgLikeKind::Audio(c),
        MessageType::File(c) => FrontendMsgLikeKind::File(c),
        MessageType::Image(c) => FrontendMsgLikeKind::Image(c),
        MessageType::Text(c) => FrontendMsgLikeKind::Text(c),
        MessageType::Video(c) => FrontendMsgLikeKind::Video(c),
        MessageType::Emote(c) => FrontendMsgLikeKind::Emote(c),
        MessageType::Location(c) => FrontendMsgLikeKind::Location(c),
        MessageType::Notice(c) => FrontendMsgLikeKind::Notice(c),
        MessageType::ServerNotice(c) => FrontendMsgLikeKind::ServerNotice(c),
        MessageType::VerificationRequest(c) => FrontendMsgLikeKind::VerificationRequest(c),
        _ => FrontendMsgLikeKind::Unknown,
    }
}

bitflags! {
    /// Possible actions that the user can perform on a message.
    ///
    /// This is used to determine which buttons to show in the message context menu.
    #[derive(Copy, Clone, Debug)]
    pub struct MessageAbilities: u8 {
        /// Whether the user can react to this message.
        const CanReact = 1 << 0;
        /// Whether the user can reply to this message.
        const CanReplyTo = 1 << 1;
        /// Whether the user can edit this message.
        const CanEdit = 1 << 2;
        /// Whether the user can pin this message.
        const CanPin = 1 << 3;
        /// Whether the user can unpin this message.
        const CanUnpin = 1 << 4;
        /// Whether the user can delete/redact this message.
        const CanDelete = 1 << 5;
    }
}
impl MessageAbilities {
    pub fn from_user_power_and_event(
        user_power_levels: &UserPowerLevels,
        event_tl_item: &EventTimelineItem,
    ) -> Self {
        let mut abilities = Self::empty();
        abilities.set(Self::CanEdit, event_tl_item.is_editable());
        // Currently we only support deleting one's own messages.
        if event_tl_item.is_own() {
            abilities.set(Self::CanDelete, user_power_levels._can_redact_own());
        }
        abilities.set(Self::CanReplyTo, event_tl_item.can_be_replied_to());
        abilities.set(Self::CanPin, user_power_levels._can_pin());
        // TODO: currently we don't differentiate between pin and unpin,
        //       but we should first check whether the given message is already pinned
        //       before deciding which ability to set.
        // abilities.set(Self::CanUnPin, user_power_levels.can_pin_unpin());
        abilities.set(Self::CanReact, user_power_levels._can_send_reaction());
        abilities
    }
}

impl Serialize for MessageAbilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;

        let mut seq = serializer.serialize_seq(None)?;

        if self.contains(MessageAbilities::CanReact) {
            seq.serialize_element("canReact")?;
        }
        if self.contains(MessageAbilities::CanReplyTo) {
            seq.serialize_element("canReplyTo")?;
        }
        if self.contains(MessageAbilities::CanEdit) {
            seq.serialize_element("canEdit")?;
        }
        if self.contains(MessageAbilities::CanPin) {
            seq.serialize_element("canPin")?;
        }
        if self.contains(MessageAbilities::CanUnpin) {
            seq.serialize_element("canUnpin")?;
        }
        if self.contains(MessageAbilities::CanDelete) {
            seq.serialize_element("canDelete")?;
        }

        seq.end()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn map_timeline_event_item_content(
    timeline_item_content: &TimelineItemContent,
    unique_id: String,
    timeline_item_id: FrontendTimelineEventItemId,
    is_own: bool,
    is_local: bool,
    timestamp: Option<UInt>,
    sender: Option<String>,
    sender_id: String,
    abilities: MessageAbilities,
    event_id: Option<OwnedEventId>,
) -> FrontendTimelineItem {
    match timeline_item_content {
        TimelineItemContent::MsgLike(msg_like) => {
            let in_reply_to_id = msg_like.in_reply_to.clone().map(|r| r.event_id);
            let thread_root = msg_like.thread_root.clone();
            let thread_summary = msg_like
                .thread_summary
                .clone()
                .and_then(get_frontend_thread_summary);
            match msg_like.kind.clone() {
                MsgLikeKind::Message(message) => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: message.is_edited(),
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: map_msg_event_content(message.msgtype().clone()),
                    }),
                },
                MsgLikeKind::Sticker(sticker) => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: false,
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: FrontendMsgLikeKind::Sticker(FrontendStickerEventContent::from(
                            sticker.content().clone(),
                        )),
                    }),
                },
                MsgLikeKind::Redacted => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: true,
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: FrontendMsgLikeKind::Redacted,
                    }),
                },
                MsgLikeKind::UnableToDecrypt(_) => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: false,
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: FrontendMsgLikeKind::UnableToDecrypt,
                    }),
                },

                MsgLikeKind::Poll(_) => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: false,
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: FrontendMsgLikeKind::Poll,
                    }),
                },

                MsgLikeKind::Other(_) => FrontendTimelineItem {
                    unique_id,
                    event_id,
                    timeline_item_id,
                    is_local,
                    is_own,
                    timestamp,
                    abilities,
                    data: FrontendTimelineItemData::MsgLike(FrontendMsgLikeContent {
                        edited: false,
                        reactions: msg_like.reactions.clone(),
                        sender_id,
                        sender,
                        thread_root,
                        thread_summary,
                        in_reply_to_id,
                        kind: FrontendMsgLikeKind::Unknown,
                    }),
                },
            }
        }
        TimelineItemContent::OtherState(state) => FrontendTimelineItem {
            unique_id,
            event_id,
            timeline_item_id,
            is_local,
            is_own,
            timestamp,
            abilities,
            data: FrontendTimelineItemData::StateChange(FrontendStateEvent::OtherState(
                FrontendAnyOtherFullStateEventContent::from(state.content().clone()),
            )),
        },
        TimelineItemContent::MembershipChange(change) => FrontendTimelineItem {
            unique_id,
            event_id,
            timeline_item_id,
            is_local,
            is_own,
            timestamp,
            abilities,
            data: FrontendTimelineItemData::StateChange(FrontendStateEvent::MembershipChange(
                FrontendRoomMembershipChange::from(change.clone()),
            )),
        },

        TimelineItemContent::ProfileChange(change) => FrontendTimelineItem {
            unique_id,
            event_id,
            timeline_item_id,
            is_local,
            is_own,
            timestamp,
            abilities,
            data: FrontendTimelineItemData::StateChange(FrontendStateEvent::ProfileChange(
                FrontendMemberProfileChange::from(change.clone()),
            )),
        },

        TimelineItemContent::RtcNotification | TimelineItemContent::CallInvite => {
            FrontendTimelineItem {
                unique_id,
                event_id,
                timeline_item_id,
                is_local,
                is_own,
                timestamp,
                abilities,
                data: FrontendTimelineItemData::Call,
            }
        }

        TimelineItemContent::FailedToParseMessageLike {
            event_type: _,
            error,
        } => FrontendTimelineItem {
            unique_id,
            event_id: None,
            timeline_item_id,
            data: FrontendTimelineItemData::Error(FrontendTimelineErrorItem {
                error: error.to_string(),
            }),
            is_local: true,
            is_own: true,
            timestamp: None,
            abilities,
        },

        TimelineItemContent::FailedToParseState {
            state_key: _,
            event_type: _,
            error,
        } => FrontendTimelineItem {
            unique_id,
            event_id: None,
            timeline_item_id,
            data: FrontendTimelineItemData::Error(FrontendTimelineErrorItem {
                error: error.to_string(),
            }),
            is_local: true,
            is_own: true,
            timestamp: None,
            abilities,
        },
    }
}
