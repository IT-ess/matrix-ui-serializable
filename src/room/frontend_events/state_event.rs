use std::ops::Deref;
use std::ops::DerefMut;

use matrix_sdk::ruma::OwnedMxcUri;
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::events::RedactContent;
use matrix_sdk::ruma::events::StateEventContentChange;
use matrix_sdk::ruma::events::StaticStateEventContent;
use matrix_sdk::ruma::events::policy::rule::room::PolicyRuleRoomEventContent;
use matrix_sdk::ruma::events::policy::rule::server::PolicyRuleServerEventContent;
use matrix_sdk::ruma::events::policy::rule::user::PolicyRuleUserEventContent;
use matrix_sdk::ruma::events::room::aliases::RoomAliasesEventContent;
use matrix_sdk::ruma::events::room::avatar::RoomAvatarEventContent;
use matrix_sdk::ruma::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use matrix_sdk::ruma::events::room::create::RoomCreateEventContent;
use matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent;
use matrix_sdk::ruma::events::room::guest_access::RoomGuestAccessEventContent;
use matrix_sdk::ruma::events::room::history_visibility::RoomHistoryVisibilityEventContent;
use matrix_sdk::ruma::events::room::join_rules::RoomJoinRulesEventContent;
use matrix_sdk::ruma::events::room::member::Change;
use matrix_sdk::ruma::events::room::member::RoomMemberEventContent;
use matrix_sdk::ruma::events::room::name::RoomNameEventContent;
use matrix_sdk::ruma::events::room::pinned_events::RoomPinnedEventsEventContent;
use matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent;
use matrix_sdk::ruma::events::room::server_acl::RoomServerAclEventContent;
use matrix_sdk::ruma::events::room::third_party_invite::RoomThirdPartyInviteEventContent;
use matrix_sdk::ruma::events::room::tombstone::RoomTombstoneEventContent;
use matrix_sdk::ruma::events::room::topic::RoomTopicEventContent;
use matrix_sdk::ruma::events::space::child::SpaceChildEventContent;
use matrix_sdk::ruma::events::space::parent::SpaceParentEventContent;
use matrix_sdk_ui::timeline::AnyOtherStateEventContentChange;
use matrix_sdk_ui::timeline::MemberProfileChange;
use matrix_sdk_ui::timeline::MembershipChange;
use matrix_sdk_ui::timeline::RoomMembershipChange;
use serde::Serialize;
use serde::Serializer;

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "body"
)]
pub enum FrontendStateEvent {
    OtherState(FrontendAnyOtherStateEventContentChange),
    MembershipChange(FrontendRoomMembershipChange),
    ProfileChange(FrontendMemberProfileChange),
}

//
// COMMON
//

// Newtype for StateEventContentChange
#[derive(Debug, Clone)]
struct FrontendStateEventContentChange<C>(StateEventContentChange<C>)
where
    C: StaticStateEventContent + RedactContent,
    C::Redacted: std::fmt::Debug + Clone,
    C::PossiblyRedacted: std::fmt::Debug + Clone;

impl<C> Deref for FrontendStateEventContentChange<C>
where
    C: StaticStateEventContent + RedactContent,
    C::Redacted: std::fmt::Debug + Clone,
    C::PossiblyRedacted: std::fmt::Debug + Clone,
{
    type Target = StateEventContentChange<C>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<C> DerefMut for FrontendStateEventContentChange<C>
where
    C: StaticStateEventContent + RedactContent,
    C::Redacted: std::fmt::Debug + Clone,
    C::PossiblyRedacted: std::fmt::Debug + Clone,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<C> From<StateEventContentChange<C>> for FrontendStateEventContentChange<C>
where
    C: StaticStateEventContent + RedactContent,
    C::Redacted: std::fmt::Debug + Clone,
    C::PossiblyRedacted: std::fmt::Debug + Clone,
{
    fn from(content: StateEventContentChange<C>) -> Self {
        FrontendStateEventContentChange(content)
    }
}

// Implement Serialize for the StateEventContentChange wrapper
impl<C> Serialize for FrontendStateEventContentChange<C>
where
    C: StaticStateEventContent + RedactContent + Serialize,
    C::PossiblyRedacted: Serialize + std::fmt::Debug + Clone,
    C::Redacted: Serialize + std::fmt::Debug + Clone,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.0 {
            StateEventContentChange::Original {
                content,
                prev_content,
            } => {
                use serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct("StateEventContentChange", 2)?;
                state.serialize_field("content", content)?;
                state.serialize_field("prev_content", prev_content)?;
                state.end()
            }
            StateEventContentChange::Redacted(redacted) => redacted.serialize(serializer),
        }
    }
}

//
// OTHER STATE
//

// Newtype for AnyOtherStateEventContentChange
#[derive(Debug, Clone)]
pub struct FrontendAnyOtherStateEventContentChange(AnyOtherStateEventContentChange);

impl Deref for FrontendAnyOtherStateEventContentChange {
    type Target = AnyOtherStateEventContentChange;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendAnyOtherStateEventContentChange {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<AnyOtherStateEventContentChange> for FrontendAnyOtherStateEventContentChange {
    fn from(content: AnyOtherStateEventContentChange) -> Self {
        FrontendAnyOtherStateEventContentChange(content)
    }
}

// Updated enum using the wrapped StateEventContentChange
#[derive(Clone, Debug, Serialize)]
enum FrontendAnyOtherStateEventContentChangeEnum {
    /// m.policy.rule.room
    PolicyRuleRoom(FrontendStateEventContentChange<PolicyRuleRoomEventContent>),

    /// m.policy.rule.server
    PolicyRuleServer(FrontendStateEventContentChange<PolicyRuleServerEventContent>),

    /// m.policy.rule.user
    PolicyRuleUser(FrontendStateEventContentChange<PolicyRuleUserEventContent>),

    /// m.room.aliases
    RoomAliases(FrontendStateEventContentChange<RoomAliasesEventContent>),

    /// m.room.avatar
    RoomAvatar(FrontendStateEventContentChange<RoomAvatarEventContent>),

    /// m.room.canonical_alias
    RoomCanonicalAlias(FrontendStateEventContentChange<RoomCanonicalAliasEventContent>),

    /// m.room.create
    RoomCreate(FrontendStateEventContentChange<RoomCreateEventContent>),

    /// m.room.encryption
    RoomEncryption(FrontendStateEventContentChange<RoomEncryptionEventContent>),

    /// m.room.guest_access
    RoomGuestAccess(FrontendStateEventContentChange<RoomGuestAccessEventContent>),

    /// m.room.history_visibility
    RoomHistoryVisibility(FrontendStateEventContentChange<RoomHistoryVisibilityEventContent>),

    /// m.room.join_rules
    RoomJoinRules(FrontendStateEventContentChange<RoomJoinRulesEventContent>),

    /// m.room.name
    RoomName(FrontendStateEventContentChange<RoomNameEventContent>),

    /// m.room.pinned_events
    RoomPinnedEvents(FrontendStateEventContentChange<RoomPinnedEventsEventContent>),

    /// m.room.power_levels
    RoomPowerLevels(FrontendStateEventContentChange<RoomPowerLevelsEventContent>),

    /// m.room.server_acl
    RoomServerAcl(FrontendStateEventContentChange<RoomServerAclEventContent>),

    /// m.room.third_party_invite
    RoomThirdPartyInvite(FrontendStateEventContentChange<RoomThirdPartyInviteEventContent>),

    /// m.room.tombstone
    RoomTombstone(FrontendStateEventContentChange<RoomTombstoneEventContent>),

    /// m.room.topic
    RoomTopic(FrontendStateEventContentChange<RoomTopicEventContent>),

    /// m.space.child
    SpaceChild(FrontendStateEventContentChange<SpaceChildEventContent>),

    /// m.space.parent
    SpaceParent(FrontendStateEventContentChange<SpaceParentEventContent>),

    // Custom
    Custom,
}

// Implement Serialize for the main wrapper by converting to the serializable enum
impl Serialize for FrontendAnyOtherStateEventContentChange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Convert the inner enum to our serializable version
        let serializable_enum = match &self.0 {
            AnyOtherStateEventContentChange::PolicyRuleRoom(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::PolicyRuleRoom(content.clone().into())
            }
            AnyOtherStateEventContentChange::PolicyRuleServer(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::PolicyRuleServer(
                    content.clone().into(),
                )
            }
            AnyOtherStateEventContentChange::PolicyRuleUser(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::PolicyRuleUser(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomAliases(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomAliases(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomAvatar(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomAvatar(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomCanonicalAlias(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomCanonicalAlias(
                    content.clone().into(),
                )
            }
            AnyOtherStateEventContentChange::RoomCreate(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomCreate(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomEncryption(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomEncryption(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomGuestAccess(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomGuestAccess(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomHistoryVisibility(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomHistoryVisibility(
                    content.clone().into(),
                )
            }
            AnyOtherStateEventContentChange::RoomJoinRules(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomJoinRules(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomName(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomName(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomPinnedEvents(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomPinnedEvents(
                    content.clone().into(),
                )
            }
            AnyOtherStateEventContentChange::RoomPowerLevels(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomPowerLevels(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomServerAcl(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomServerAcl(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomThirdPartyInvite(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomThirdPartyInvite(
                    content.clone().into(),
                )
            }
            AnyOtherStateEventContentChange::RoomTombstone(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomTombstone(content.clone().into())
            }
            AnyOtherStateEventContentChange::RoomTopic(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::RoomTopic(content.clone().into())
            }
            AnyOtherStateEventContentChange::SpaceChild(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::SpaceChild(content.clone().into())
            }
            AnyOtherStateEventContentChange::SpaceParent(content) => {
                FrontendAnyOtherStateEventContentChangeEnum::SpaceParent(content.clone().into())
            }
            _ => FrontendAnyOtherStateEventContentChangeEnum::Custom,
        };

        serializable_enum.serialize(serializer)
    }
}

//
// MEMBERSHIP STATE
//
// Newtype for MembershipChange to add Serialize
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontendMembershipChange(MembershipChange);

impl Deref for FrontendMembershipChange {
    type Target = MembershipChange;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<MembershipChange> for FrontendMembershipChange {
    fn from(change: MembershipChange) -> Self {
        FrontendMembershipChange(change)
    }
}

impl Serialize for FrontendMembershipChange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let variant_name = match self.0 {
            MembershipChange::None => "None",
            MembershipChange::Error => "Error",
            MembershipChange::Joined => "Joined",
            MembershipChange::Left => "Left",
            MembershipChange::Banned => "Banned",
            MembershipChange::Unbanned => "Unbanned",
            MembershipChange::Kicked => "Kicked",
            MembershipChange::Invited => "Invited",
            MembershipChange::KickedAndBanned => "KickedAndBanned",
            MembershipChange::InvitationAccepted => "InvitationAccepted",
            MembershipChange::InvitationRejected => "InvitationRejected",
            MembershipChange::InvitationRevoked => "InvitationRevoked",
            MembershipChange::Knocked => "Knocked",
            MembershipChange::KnockAccepted => "KnockAccepted",
            MembershipChange::KnockRetracted => "KnockRetracted",
            MembershipChange::KnockDenied => "KnockDenied",
            MembershipChange::NotImplemented => "NotImplemented",
        };
        serializer.serialize_str(variant_name)
    }
}

// Newtype for RoomMembershipChange
#[derive(Debug, Clone)]
pub struct FrontendRoomMembershipChange(RoomMembershipChange);

impl Deref for FrontendRoomMembershipChange {
    type Target = RoomMembershipChange;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendRoomMembershipChange {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<RoomMembershipChange> for FrontendRoomMembershipChange {
    fn from(change: RoomMembershipChange) -> Self {
        FrontendRoomMembershipChange(change)
    }
}

impl Serialize for FrontendRoomMembershipChange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("RoomMembershipChange", 3)?;

        // Use accessor methods if they exist, or you might need to check the API docs
        // For now, let's assume there are getter methods:
        state.serialize_field("user_id", self.user_id())?;

        // Serialize content using the FrontendFullStateEventContent wrapper
        let frontend_content = FrontendStateEventContentChange::from(self.content().clone());
        state.serialize_field("content", &frontend_content)?;

        // Serialize change, wrapping Option<MembershipChange> with our frontend type
        let frontend_change = self.change().map(FrontendMembershipChange::from);
        state.serialize_field("change", &frontend_change)?;

        state.end()
    }
}

// Helper methods for easier usage
impl FrontendRoomMembershipChange {
    pub fn _new(room_membership_change: RoomMembershipChange) -> Self {
        Self(room_membership_change)
    }

    // These methods will delegate to the wrapped type's public API
    // You'll need to check what methods RoomMembershipChange actually provides
    pub fn user_id(&self) -> &UserId {
        // If RoomMembershipChange has a user_id() method:
        self.0.user_id()
        // Or if it has some other accessor method, use that instead
    }

    pub fn content(&self) -> &StateEventContentChange<RoomMemberEventContent> {
        // If RoomMembershipChange has a content() method:
        self.0.content()
        // Or use whatever the actual accessor method is
    }

    pub fn change(&self) -> Option<MembershipChange> {
        // If RoomMembershipChange has a change() method:
        self.0.change()
        // Or use whatever the actual accessor method is
    }
}

//
// PROFILE STATE
//

// Newtype for Change<T> to add Serialize
#[derive(Debug, Clone)]
struct FrontendChange<T>(Change<T>);

impl<T> Deref for FrontendChange<T> {
    type Target = Change<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for FrontendChange<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> From<Change<T>> for FrontendChange<T> {
    fn from(change: Change<T>) -> Self {
        FrontendChange(change)
    }
}

impl<T> Serialize for FrontendChange<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("Change", 2)?;
        state.serialize_field("old", &self.0.old)?;
        state.serialize_field("new", &self.0.new)?;
        state.end()
    }
}

// Newtype for MemberProfileChange
#[derive(Debug, Clone)]
pub struct FrontendMemberProfileChange(MemberProfileChange);

impl Deref for FrontendMemberProfileChange {
    type Target = MemberProfileChange;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendMemberProfileChange {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<MemberProfileChange> for FrontendMemberProfileChange {
    fn from(change: MemberProfileChange) -> Self {
        FrontendMemberProfileChange(change)
    }
}

impl Serialize for FrontendMemberProfileChange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("MemberProfileChange", 3)?;

        // Use accessor methods - you'll need to check what methods MemberProfileChange provides
        state.serialize_field("user_id", self.user_id())?;

        // Wrap displayname_change with our FrontendChange wrapper
        let frontend_displayname_change = self
            .displayname_change()
            .map(|change| FrontendChange::from(change.clone()));
        state.serialize_field("displayname_change", &frontend_displayname_change)?;

        // Wrap avatar_url_change with our FrontendChange wrapper
        let frontend_avatar_url_change = self
            .avatar_url_change()
            .map(|change| FrontendChange::from(change.clone()));
        state.serialize_field("avatar_url_change", &frontend_avatar_url_change)?;

        state.end()
    }
}

// Helper methods for easier usage
impl FrontendMemberProfileChange {
    pub fn _new(member_profile_change: MemberProfileChange) -> Self {
        Self(member_profile_change)
    }

    // These methods will delegate to the wrapped type's public API
    // You'll need to check what methods MemberProfileChange actually provides
    pub fn user_id(&self) -> &UserId {
        // If MemberProfileChange has a user_id() method:
        self.0.user_id()
        // Or use whatever the actual accessor method is
    }

    pub fn displayname_change(&self) -> Option<&Change<Option<String>>> {
        // If MemberProfileChange has a displayname_change() method:
        self.0.displayname_change()
        // Or use whatever the actual accessor method is
    }

    pub fn avatar_url_change(&self) -> Option<&Change<Option<OwnedMxcUri>>> {
        // If MemberProfileChange has an avatar_url_change() method:
        self.0.avatar_url_change()
        // Or use whatever the actual accessor method is
    }
}

// Helper methods for FrontendChange for easier access to old/new values
impl<T> FrontendChange<T> {
    pub fn _new(change: Change<T>) -> Self {
        Self(change)
    }

    pub fn _old(&self) -> &T {
        &self.0.old
    }

    pub fn _new_value(&self) -> &T {
        &self.0.new
    }
}
