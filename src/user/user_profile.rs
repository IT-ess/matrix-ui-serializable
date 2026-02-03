use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    sync::{Arc, LazyLock},
};

use crossbeam_queue::SegQueue;
use matrix_sdk::{
    room::RoomMember,
    ruma::{OwnedMxcUri, OwnedRoomId, OwnedUserId},
};
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::warn;

use crate::{
    models::{
        async_requests::{MatrixRequest, submit_async_request},
        events::{ToastNotificationRequest, ToastNotificationVariant},
        state_updater::StateUpdater,
    },
    room::notifications::enqueue_toast_notification,
};

use crate::init::singletons::{UIUpdateMessage, broadcast_event};

/// Information retrieved about a user: their displayable name, ID, and known avatar state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub user_id: OwnedUserId,
    /// The user's default display name, if set.
    /// Note that a user may have per-room display names,
    /// so this should be considered a fallback.
    pub username: Option<String>,
    pub avatar_url: Option<OwnedMxcUri>,
}
impl UserProfile {
    /// Returns the user's displayable name, using the user ID as a fallback.
    pub fn _displayable_name(&self) -> &str {
        if let Some(un) = self.username.as_ref()
            && !un.is_empty()
        {
            return un.as_str();
        }
        self.user_id.as_str()
    }
}

/// The queue of user profile updates waiting to be processed by the UI thread's event handler.
static PENDING_USER_PROFILE_UPDATES: SegQueue<UserProfileUpdate> = SegQueue::new();

/// Enqueues a new user profile update and signals the UI that an update is available.
pub fn enqueue_user_profile_update(update: UserProfileUpdate) {
    PENDING_USER_PROFILE_UPDATES.push(update);
    broadcast_event(UIUpdateMessage::RefreshUI);
}

/// A user profile update, which can include changes to a user's full profile
/// and/or room membership info.
pub enum UserProfileUpdate {
    /// A fully-fetched user profile, with info about the user's membership in a given room.
    Full {
        new_profile: UserProfile,
        room_id: OwnedRoomId,
        _room_member: RoomMember,
    },
    /// An update to the user's room membership info only, without any profile changes.
    RoomMemberOnly {
        room_id: OwnedRoomId,
        room_member: RoomMember,
    },
    /// An update to the user's profile only, without changes to room membership info.
    UserProfileOnly(UserProfile),
}

impl UserProfileUpdate {
    /// Applies this update to the given user profile info cache.
    fn apply_to_cache(self, cache: &mut BTreeMap<OwnedUserId, UserProfileCacheEntry>) {
        match self {
            UserProfileUpdate::Full {
                new_profile,
                room_id,
                _room_member: _,
            } => match cache.entry(new_profile.user_id.clone()) {
                Entry::Occupied(mut entry) => match entry.get_mut() {
                    e @ UserProfileCacheEntry::Requested => {
                        *e = UserProfileCacheEntry::Loaded {
                            user_profile: new_profile,
                            rooms: {
                                let mut rooms = BTreeSet::new();
                                rooms.insert(room_id);
                                rooms
                            },
                        };
                    }
                    UserProfileCacheEntry::Loaded {
                        user_profile,
                        rooms,
                    } => {
                        *user_profile = new_profile;
                        rooms.insert(room_id);
                    }
                },
                Entry::Vacant(entry) => {
                    entry.insert(UserProfileCacheEntry::Loaded {
                        user_profile: new_profile,
                        rooms: {
                            let mut rooms = BTreeSet::new();
                            rooms.insert(room_id);
                            rooms
                        },
                    });
                }
            },
            UserProfileUpdate::RoomMemberOnly {
                room_id,
                room_member,
            } => {
                match cache.entry(room_member.user_id().to_owned()) {
                    Entry::Occupied(mut entry) => match entry.get_mut() {
                        e @ UserProfileCacheEntry::Requested => {
                            // This shouldn't happen, but we can still technically handle it correctly.
                            warn!(
                                "BUG: User profile cache entry was `Requested` for user {} when handling RoomMemberOnly update",
                                room_member.user_id()
                            );
                            *e = UserProfileCacheEntry::Loaded {
                                user_profile: UserProfile {
                                    user_id: room_member.user_id().to_owned(),
                                    username: None,
                                    avatar_url: room_member.avatar_url().map(|url| url.to_owned()),
                                },
                                rooms: {
                                    let mut rooms = BTreeSet::new();
                                    rooms.insert(room_id);
                                    rooms
                                },
                            };
                        }
                        UserProfileCacheEntry::Loaded { rooms, .. } => {
                            rooms.insert(room_id);
                        }
                    },
                    Entry::Vacant(entry) => {
                        // This shouldn't happen, but we can still technically handle it correctly.
                        warn!(
                            "BUG: User profile cache entry not found for user {} when handling RoomMemberOnly update",
                            room_member.user_id()
                        );
                        entry.insert(UserProfileCacheEntry::Loaded {
                            user_profile: UserProfile {
                                user_id: room_member.user_id().to_owned(),
                                username: None,
                                avatar_url: room_member.avatar_url().map(|url| url.to_owned()),
                            },
                            rooms: {
                                let mut rooms = BTreeSet::new();
                                rooms.insert(room_id);
                                rooms
                            },
                        });
                    }
                }
            }
            UserProfileUpdate::UserProfileOnly(new_profile) => {
                match cache.entry(new_profile.user_id.clone()) {
                    Entry::Occupied(mut entry) => match entry.get_mut() {
                        e @ UserProfileCacheEntry::Requested => {
                            *e = UserProfileCacheEntry::Loaded {
                                user_profile: new_profile,
                                rooms: BTreeSet::new(),
                            };
                        }
                        UserProfileCacheEntry::Loaded { user_profile, .. } => {
                            *user_profile = new_profile;
                        }
                    },
                    Entry::Vacant(entry) => {
                        entry.insert(UserProfileCacheEntry::Loaded {
                            user_profile: new_profile,
                            rooms: BTreeSet::new(),
                        });
                    }
                }
            }
        }
    }
}

/// A cache of each user's profile and the rooms they are a member of, indexed by user ID.
#[derive(Debug, Clone, Serialize)]
pub struct UserProfileMap(BTreeMap<OwnedUserId, UserProfileCacheEntry>);

/// A cache of each user's profile and the rooms they are a member of, indexed by user ID.
static USER_PROFILE_CACHE: LazyLock<RwLock<UserProfileMap>> =
    LazyLock::new(|| RwLock::new(UserProfileMap(BTreeMap::new())));

/// Processes all pending user profile updates in the queue.
pub async fn process_user_profile_updates(updaters: &Arc<Box<dyn StateUpdater>>) -> bool {
    let mut updated = false;
    if PENDING_USER_PROFILE_UPDATES.is_empty() {
        return updated; // Return early if the queue is empty to avoid acquiring the lock.
    }
    {
        let mut lock = USER_PROFILE_CACHE.write().await;
        while let Some(update) = PENDING_USER_PROFILE_UPDATES.pop() {
            // Insert the updated info into the cache
            update.apply_to_cache(&mut lock.0);
            updated = true;
        }
    } // We drop the write lock here
    if updated {
        let lock = USER_PROFILE_CACHE.read().await;
        if let Err(e) = updaters.update_profile(&lock) {
            enqueue_toast_notification(ToastNotificationRequest::new(
                format!("Cannot update profiles store. Error: {e}"),
                None,
                ToastNotificationVariant::Error,
            ))
        }
    }
    updated
}

/// Submit a request to retrieve the user profile or returns true if the entry is already requested.
pub async fn fetch_user_profile(user_id: OwnedUserId, room_id: Option<OwnedRoomId>) -> bool {
    let mut lock = USER_PROFILE_CACHE.write().await;
    match lock.0.entry(user_id) {
        Entry::Occupied(_) => true,
        Entry::Vacant(entry) => {
            submit_async_request(MatrixRequest::GetUserProfile {
                user_id: entry.key().clone(),
                room_id,
                local_only: false,
            });
            entry.insert(UserProfileCacheEntry::Requested);
            false
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state",
    content = "data"
)]
enum UserProfileCacheEntry {
    /// A request has been issued and we're waiting for it to complete.
    Requested,
    /// The profile has been successfully loaded from the server.
    Loaded {
        #[serde(flatten)]
        user_profile: UserProfile,
        rooms: BTreeSet<OwnedRoomId>,
    },
}
