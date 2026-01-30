use std::ops::{Deref, DerefMut};

use bitflags::bitflags;
use matrix_sdk::{
    Room,
    ruma::{
        UserId,
        events::{
            MessageLikeEventType, StateEventType,
            room::power_levels::{RoomPowerLevels, UserPowerLevel},
        },
    },
};
use serde::{Serialize, Serializer};

bitflags! {
    /// The powers that a user has in a given room.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub struct UserPowerLevels: u64 {
        const Ban = 1 << 0;
        const Invite = 1 << 1;
        const Kick = 1 << 2;
        const Redact = 1 << 3;
        const NotifyRoom = 1 << 4;
        // -------------------------------------
        // -- Copied from TimelineEventType ----
        // -- Unused powers are commented out --
        // -------------------------------------
        // const CallAnswer = 1 << 5;
        // const CallInvite = 1 << 6;
        // const CallHangup = 1 << 7;
        // const CallCandidates = 1 << 8;
        // const CallNegotiate = 1 << 9;
        // const CallReject = 1 << 10;
        // const CallSdpStreamMetadataChanged = 1 << 11;
        // const CallSelectAnswer = 1 << 12;
        // const KeyVerificationReady = 1 << 13;
        // const KeyVerificationStart = 1 << 14;
        // const KeyVerificationCancel = 1 << 15;
        // const KeyVerificationAccept = 1 << 16;
        // const KeyVerificationKey = 1 << 17;
        // const KeyVerificationMac = 1 << 18;
        // const KeyVerificationDone = 1 << 19;
        const Location = 1 << 20;
        const Message = 1 << 21;
        // const PollStart = 1 << 22;
        // const UnstablePollStart = 1 << 23;
        // const PollResponse = 1 << 24;
        // const UnstablePollResponse = 1 << 25;
        // const PollEnd = 1 << 26;
        // const UnstablePollEnd = 1 << 27;
        // const Beacon = 1 << 28;
        const Reaction = 1 << 29;
        // const RoomEncrypted = 1 << 30;
        const RoomMessage = 1 << 31;
        const RoomRedaction = 1 << 32;
        const Sticker = 1 << 33;
        // const CallNotify = 1 << 34;
        // const PolicyRuleRoom = 1 << 35;
        // const PolicyRuleServer = 1 << 36;
        // const PolicyRuleUser = 1 << 37;
        // const RoomAliases = 1 << 38;
        const RoomAvatar = 1 << 39;
        // const RoomCanonicalAlias = 1 << 40;
        // const RoomCreate = 1 << 41;
        // const RoomEncryption = 1 << 42;
        // const RoomGuestAccess = 1 << 43;
        // const RoomHistoryVisibility = 1 << 44;
        // const RoomJoinRules = 1 << 45;
        // const RoomMember = 1 << 46;
        const RoomName = 1 << 47;
        const RoomPinnedEvents = 1 << 48;
        // const RoomPowerLevels = 1 << 49;
        // const RoomServerAcl = 1 << 50;
        // const RoomThirdPartyInvite = 1 << 51;
        // const RoomTombstone = 1 << 52;
        const RoomTopic = 1 << 53;
        // const SpaceChild = 1 << 54;
        // const SpaceParent = 1 << 55;
        // const BeaconInfo = 1 << 56;
        // const CallMember = 1 << 57;
        // const MemberHints = 1 << 58;
    }
}
impl UserPowerLevels {
    pub async fn from_room(room: &Room, user_id: &UserId) -> Option<Self> {
        let room_power_levels = room.power_levels().await.ok()?;
        Some(UserPowerLevels::from(&room_power_levels, user_id))
    }

    pub fn from(power_levels: &RoomPowerLevels, user_id: &UserId) -> Self {
        let mut retval = UserPowerLevels::empty();
        let user_power = power_levels.for_user(user_id);
        retval.set(UserPowerLevels::Ban, user_power >= power_levels.ban);
        retval.set(UserPowerLevels::Invite, user_power >= power_levels.invite);
        retval.set(UserPowerLevels::Kick, user_power >= power_levels.kick);
        retval.set(UserPowerLevels::Redact, user_power >= power_levels.redact);
        retval.set(
            UserPowerLevels::NotifyRoom,
            user_power >= power_levels.notifications.room,
        );
        retval.set(
            UserPowerLevels::Location,
            user_power >= power_levels.for_message(MessageLikeEventType::Location),
        );
        retval.set(
            UserPowerLevels::Message,
            user_power >= power_levels.for_message(MessageLikeEventType::Message),
        );
        retval.set(
            UserPowerLevels::Reaction,
            user_power >= power_levels.for_message(MessageLikeEventType::Reaction),
        );
        retval.set(
            UserPowerLevels::RoomMessage,
            user_power >= power_levels.for_message(MessageLikeEventType::RoomMessage),
        );
        retval.set(
            UserPowerLevels::RoomRedaction,
            user_power >= power_levels.for_message(MessageLikeEventType::RoomRedaction),
        );
        retval.set(
            UserPowerLevels::Sticker,
            user_power >= power_levels.for_message(MessageLikeEventType::Sticker),
        );
        retval.set(
            UserPowerLevels::RoomAvatar,
            user_power >= power_levels.for_state(StateEventType::RoomAvatar),
        );
        retval.set(
            UserPowerLevels::RoomName,
            user_power >= power_levels.for_state(StateEventType::RoomName),
        );
        retval.set(
            UserPowerLevels::RoomPinnedEvents,
            user_power >= power_levels.for_state(StateEventType::RoomPinnedEvents),
        );
        retval.set(
            UserPowerLevels::RoomTopic,
            user_power >= power_levels.for_state(StateEventType::RoomTopic),
        );
        retval
    }

    pub fn _can_ban(self) -> bool {
        self.contains(UserPowerLevels::Ban)
    }

    pub fn _can_unban(self) -> bool {
        self._can_ban() && self._can_kick()
    }

    pub fn _can_invite(self) -> bool {
        self.contains(UserPowerLevels::Invite)
    }

    pub fn _can_kick(self) -> bool {
        self.contains(UserPowerLevels::Kick)
    }

    pub fn _can_redact(self) -> bool {
        self.contains(UserPowerLevels::Redact)
    }

    pub fn _can_notify_room(self) -> bool {
        self.contains(UserPowerLevels::NotifyRoom)
    }

    pub fn _can_redact_own(self) -> bool {
        self.contains(UserPowerLevels::RoomRedaction)
    }

    pub fn _can_redact_others(self) -> bool {
        self._can_redact_own() && self.contains(UserPowerLevels::Redact)
    }

    pub fn _can_send_location(self) -> bool {
        self.contains(UserPowerLevels::Location)
    }

    pub fn _can_send_message(self) -> bool {
        self.contains(UserPowerLevels::RoomMessage) || self.contains(UserPowerLevels::Message)
    }

    pub fn _can_send_reaction(self) -> bool {
        self.contains(UserPowerLevels::Reaction)
    }

    pub fn _can_send_sticker(self) -> bool {
        self.contains(UserPowerLevels::Sticker)
    }

    #[doc(alias("unpin"))]
    pub fn _can_pin(self) -> bool {
        self.contains(UserPowerLevels::RoomPinnedEvents)
    }
}

impl Serialize for UserPowerLevels {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;

        let mut seq = serializer.serialize_seq(None)?;
        if self.contains(UserPowerLevels::Ban) {
            seq.serialize_element("ban")?;
        }
        if self.contains(UserPowerLevels::Invite) {
            seq.serialize_element("invite")?;
        }
        if self.contains(UserPowerLevels::Kick) {
            seq.serialize_element("kick")?;
        }
        if self.contains(UserPowerLevels::Redact) {
            seq.serialize_element("redact")?;
        }
        if self.contains(UserPowerLevels::NotifyRoom) {
            seq.serialize_element("notifyRoom")?;
        }

        if self.contains(UserPowerLevels::Location) {
            seq.serialize_element("location")?;
        }
        if self.contains(UserPowerLevels::Message) {
            seq.serialize_element("message")?;
        }
        if self.contains(UserPowerLevels::Reaction) {
            seq.serialize_element("reaction")?;
        }
        if self.contains(UserPowerLevels::RoomMessage) {
            seq.serialize_element("roomMessage")?;
        }
        if self.contains(UserPowerLevels::RoomRedaction) {
            seq.serialize_element("roomRedaction")?;
        }
        if self.contains(UserPowerLevels::Sticker) {
            seq.serialize_element("sticker")?;
        }
        if self.contains(UserPowerLevels::RoomAvatar) {
            seq.serialize_element("roomAvatar")?;
        }
        if self.contains(UserPowerLevels::RoomName) {
            seq.serialize_element("roomName")?;
        }
        if self.contains(UserPowerLevels::RoomPinnedEvents) {
            seq.serialize_element("roomPinnedEvents")?;
        }
        if self.contains(UserPowerLevels::RoomTopic) {
            seq.serialize_element("roomTopic")?;
        }

        seq.end()
    }
}

// New type pattern to add the msgtype field to serialization
#[derive(Debug, Clone)]
pub struct FrontendUserPowerLevel(UserPowerLevel);

impl Deref for FrontendUserPowerLevel {
    type Target = UserPowerLevel;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendUserPowerLevel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<UserPowerLevel> for FrontendUserPowerLevel {
    fn from(lvl: UserPowerLevel) -> Self {
        FrontendUserPowerLevel(lvl)
    }
}

impl Serialize for FrontendUserPowerLevel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("FrontendUserPowerLevel", 1)?;

        match **self {
            UserPowerLevel::Infinite => {
                state.serialize_field("userPowerLevel", &true)?;
            }
            UserPowerLevel::Int(i) => {
                state.serialize_field("userPowerLevel", &i)?;
            }
            _ => {
                state.serialize_field("userPowerLevel", &0)?;
            }
        }

        state.end()
    }
}
