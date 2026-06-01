//! A cache of room previews keyed by room ID or alias.
//!
//! The cache currently just stores part of the room preview info:
//! the room's ID, display name, and avatar.
//!
//! Currently we treat cache entries as stale after 24 hours, but this
//! is a placeholder for proper invalidation based on subscribing to
//! updates for any rooms in the cache.
//! Most of this implementation comes from Robrix.

use matrix_sdk::{
    RoomHero, RoomState,
    room_preview::RoomPreview,
    ruma::{
        OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId, RoomOrAliasId,
        room::{JoinRuleSummary, RoomType},
    },
};
use serde::Serialize;

use crossbeam_queue::SegQueue;
use hashbrown::hash_map::{HashMap, RawEntryMut};
use matrix_sdk::OwnedServerName;
use std::{
    sync::{LazyLock, RwLock},
    time::{Duration, Instant},
};

use crate::{
    MatrixRequest,
    init::singletons::{UIUpdateMessage, broadcast_event},
    submit_async_request,
};

const CACHE_ENTRY_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// A cache of resolved room previews, indexed by room-or-alias ID.
static ROOM_PREVIEW_CACHE: LazyLock<RwLock<HashMap<OwnedRoomOrAliasId, CacheEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

struct CacheEntry {
    state: CacheEntryState,
    /// When this entry was inserted into the cache.
    loaded_at: Instant,
}

#[allow(clippy::large_enum_variant)]
enum CacheEntryState {
    /// A fetch has been issued and we're waiting for it to complete.
    Requested,
    /// The room preview has been successfully loaded.
    Loaded { preview: SerializableRoomPreview },
}

/// A room preview entry as returned to the caller of [`get_or_fetch_room_preview`].
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CachedRoomPreview {
    /// The preview is loaded and ready to display.
    Loaded { preview: SerializableRoomPreview },
    /// A fetch is in flight; show a fallback while waiting.
    Requested,
}

/// An update sent from the matrix worker thread once a room preview fetch completes.
///
/// Carries the full [`FetchedRoomPreview`] so we can later widen what the
/// cache stores without changing the worker side.
pub struct RoomPreviewUpdate {
    pub room_or_alias_id: OwnedRoomOrAliasId,
    pub preview: SerializableRoomPreview,
}

/// The queue of room preview updates waiting to be processed by the UI thread's event handler.
static PENDING_ROOM_PREVIEW_UPDATES: SegQueue<RoomPreviewUpdate> = SegQueue::new();

/// Enqueues a new room preview update and signals the UI that an update is available.
pub fn enqueue_room_preview_update(update: RoomPreviewUpdate) {
    PENDING_ROOM_PREVIEW_UPDATES.push(update);
    broadcast_event(UIUpdateMessage::RefreshUI);
}

/// Processes all pending room preview updates in the queue.
pub fn process_room_preview_updates() {
    let mut cache = ROOM_PREVIEW_CACHE.write().unwrap();
    while let Some(update) = PENDING_ROOM_PREVIEW_UPDATES.pop() {
        let RoomPreviewUpdate {
            room_or_alias_id,
            preview,
        } = update;
        cache.insert(
            room_or_alias_id,
            CacheEntry {
                state: CacheEntryState::Loaded { preview },
                loaded_at: Instant::now(),
            },
        );
    }
}

/// Returns the cached preview for the given room ID or alias if it exists,
/// or submits a request to fetch it from the server if it isn't already cached.
///
/// If a request has already been submitted, it will not re-submit a duplicate request
/// and will simply return `CachedRoomPreview::Requested`. If the cached entry is
/// older than `CACHE_ENTRY_LIFETIME`, a fresh fetch is submitted and the entry is
/// reset to `Requested`.
pub fn get_or_fetch_room_preview(
    room_or_alias_id: &RoomOrAliasId,
    via: &[OwnedServerName],
) -> CachedRoomPreview {
    let mut cache = ROOM_PREVIEW_CACHE.write().unwrap();
    // raw_entry_mut lets us look up by `&RoomOrAliasId` without cloning;
    // we only allocate an owned key on insert.
    match cache.raw_entry_mut().from_key(room_or_alias_id) {
        RawEntryMut::Occupied(mut occupied) => {
            // Fast path: a fresh `Loaded` entry is returned as-is.
            if let CacheEntryState::Loaded { preview } = &occupied.get().state
                && occupied.get().loaded_at.elapsed() < CACHE_ENTRY_LIFETIME
            {
                return CachedRoomPreview::Loaded {
                    preview: preview.clone(),
                };
            }
            // Otherwise it's a stale `Loaded` (refetch and overwrite) or an
            // already-in-flight `Requested` (do nothing; prior fetch will land).
            if matches!(occupied.get().state, CacheEntryState::Loaded { .. }) {
                submit_async_request(MatrixRequest::GetRoomPreview {
                    room_or_alias_id: room_or_alias_id.to_owned(),
                    via: via.to_vec(),
                });
                occupied.insert(CacheEntry {
                    state: CacheEntryState::Requested,
                    loaded_at: Instant::now(),
                });
            }
            CachedRoomPreview::Requested
        }
        RawEntryMut::Vacant(vacant) => {
            submit_async_request(MatrixRequest::GetRoomPreview {
                room_or_alias_id: room_or_alias_id.to_owned(),
                via: via.to_vec(),
            });
            vacant.insert(
                room_or_alias_id.to_owned(),
                CacheEntry {
                    state: CacheEntryState::Requested,
                    loaded_at: Instant::now(),
                },
            );
            CachedRoomPreview::Requested
        }
    }
}

/// Removes all `Requested` entries from the room preview cache,
/// allowing them to be re-fetched.
///
/// This should be called when the app transitions from offline back to online,
/// because any in-flight requests that were submitted while offline have likely
/// failed, leaving stale entries that permanently block re-fetching.
pub fn _clear_all_pending_requests() {
    let mut cache = ROOM_PREVIEW_CACHE.write().unwrap();
    cache.retain(|_, entry| !matches!(entry.state, CacheEntryState::Requested));
}

/// Clears the room preview cache.
pub fn _clear_room_preview_cache() {
    let mut cache = ROOM_PREVIEW_CACHE.write().unwrap();
    cache.clear();
}

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
