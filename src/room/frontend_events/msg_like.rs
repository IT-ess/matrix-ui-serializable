use std::ops::{Deref, DerefMut};

use indexmap::IndexMap;
use matrix_sdk::ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId,
    events::{
        room::message::{
            AudioMessageEventContent, EmoteMessageEventContent, FileMessageEventContent,
            ImageMessageEventContent, KeyVerificationRequestEventContent,
            LocationMessageEventContent, NoticeMessageEventContent,
            ServerNoticeMessageEventContent, TextMessageEventContent, VideoMessageEventContent,
        },
        sticker::{StickerEventContent, StickerMediaSource},
    },
};
use matrix_sdk_ui::timeline::{ReactionInfo, ReactionStatus, ReactionsByKeyBySender};
use serde::{Serialize, Serializer};

use crate::room::frontend_events::thread_summary::FrontendThreadSummary;

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "body"
)]
pub enum FrontendMsgLikeKind {
    /// An audio message.
    Audio(AudioMessageEventContent),

    /// An emote message.
    Emote(EmoteMessageEventContent),

    /// A file message.
    File(FileMessageEventContent),

    /// An image message.
    Image(ImageMessageEventContent),

    /// A location message.
    Location(LocationMessageEventContent),

    /// A notice message.
    Notice(NoticeMessageEventContent),

    /// A server notice message.
    ServerNotice(ServerNoticeMessageEventContent),

    /// A text message.
    Text(TextMessageEventContent),

    /// A video message.
    Video(VideoMessageEventContent),

    /// A request to initiate a key verification.
    VerificationRequest(KeyVerificationRequestEventContent),

    /// An `m.sticker` event.
    Sticker(Box<FrontendStickerEventContent>),

    /// An `m.poll.start` event.
    Poll, //(PollState), // Todo: implement poll state display

    /// A redacted message.
    Redacted,

    /// An `m.room.encrypted` event that could not be decrypted.
    UnableToDecrypt,

    /// An unknown type of message
    Unknown,
}

/// A special kind of [`super::TimelineItemContent`] that groups together
/// different room message types with their respective reactions and thread
/// information.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendMsgLikeContent {
    #[serde(flatten)]
    pub kind: FrontendMsgLikeKind,
    /// Map of user reactions to this message
    pub reactions: SerializableReactions,
    /// Event ID of the thread root, if this is a threaded message.
    pub thread_root: Option<OwnedEventId>,
    // /// Information about the thread this message is the root of, if any.
    pub thread_summary: Option<FrontendThreadSummary>,
    /// The event's id this message is replying to, if any.
    pub in_reply_to_id: Option<OwnedEventId>,
    /// Wether the event has been edited at least once
    pub edited: bool,
    /// Sender display name (could be none if not resolved yet)
    pub sender: Option<String>,
    /// Sender id of the event
    pub sender_id: String,
}

// Wrap ReactionsByKeyBySender, and implement Serialize on it

#[derive(Debug)]
pub struct SerializableReactions(pub ReactionsByKeyBySender);

impl Serialize for SerializableReactions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let converted = self
            .0
            .iter()
            .map(|(key, sender_map)| {
                (
                    key.clone(),
                    sender_map
                        .iter()
                        .map(|(uid, info)| (uid.to_string(), SerializableReactionInfo::from(info)))
                        .collect::<IndexMap<_, _>>(),
                )
            })
            .collect::<IndexMap<_, _>>();

        converted.serialize(serializer)
    }
}

#[derive(Clone, Debug)]
pub struct SerializableReactionInfo {
    pub timestamp: MilliSecondsSinceUnixEpoch,
    pub status: SerializableReactionStatus,
}

impl From<&ReactionInfo> for SerializableReactionInfo {
    fn from(info: &ReactionInfo) -> Self {
        Self {
            timestamp: info.timestamp.clone(),
            status: (&info.status).into(),
        }
    }
}

// Now derive Serialize (since both fields are serializable)
impl Serialize for SerializableReactionInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.serialize_entry("status", &self.status)?;
        map.end()
    }
}

#[derive(Clone, Debug)]
pub enum SerializableReactionStatus {
    LocalToLocal,
    LocalToRemote,
    RemoteToRemote,
}

impl From<&ReactionStatus> for SerializableReactionStatus {
    fn from(status: &ReactionStatus) -> Self {
        match status {
            ReactionStatus::LocalToLocal(_) => Self::LocalToLocal,
            ReactionStatus::LocalToRemote(_) => Self::LocalToRemote,
            ReactionStatus::RemoteToRemote(_) => Self::RemoteToRemote,
        }
    }
}

impl Serialize for SerializableReactionStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let status_str = match self {
            Self::LocalToLocal => "localToLocal",
            Self::LocalToRemote => "localToRemote",
            Self::RemoteToRemote => "remoteToRemote",
        };
        serializer.serialize_str(status_str)
    }
}

// New type pattern to add the msgtype field to serialization
#[derive(Debug, Clone)]
pub struct FrontendStickerEventContent(StickerEventContent);

impl Deref for FrontendStickerEventContent {
    type Target = StickerEventContent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendStickerEventContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<StickerEventContent> for FrontendStickerEventContent {
    fn from(content: StickerEventContent) -> Self {
        FrontendStickerEventContent(content)
    }
}

impl Serialize for FrontendStickerEventContent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("StickerEventContent", 4)?;
        state.serialize_field("body", &self.0.body)?;
        state.serialize_field("info", &self.0.info)?;
        match &self.0.source {
            StickerMediaSource::Plain(u) => state.serialize_field("url", u)?,
            StickerMediaSource::Encrypted(e) => state.serialize_field("file", e)?,
            &_ => panic!("Unknown sticker source"),
        }
        state.serialize_field("msgtype", "m.sticker")?;
        state.end()
    }
}
