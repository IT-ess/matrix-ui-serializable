use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::Deref,
    sync::{Arc, Condvar, Mutex},
};

use crate::{
    events::{
        handlers::get_latest_event_details,
        timeline::{
            PerTimelineDetails, TimelineKind, TimelineUpdate, timeline_subscriber_handler,
            update_latest_event,
        },
    },
    init::singletons::{UIUpdateMessage, broadcast_event},
    room::{
        invited_room::{InvitedRoomInfo, InviterInfo},
        rooms_list::{JoinedRoomInfo, RoomsListUpdate, enqueue_rooms_list_update},
    },
    user::user_power_level::UserPowerLevels,
};
use matrix_sdk::{
    RoomDisplayName, RoomHero, RoomState,
    event_handler::EventHandlerDropGuard,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedUserId,
        events::tag::Tags,
    },
};
use matrix_sdk_ui::{
    RoomListService, Timeline,
    timeline::{RoomExt, TimelineFocus},
};
use tokio::{runtime::Handle, sync::watch};
use tracing::{debug, error, info, warn};

/// Backend-specific details about a joined room that our client currently knows about.
pub struct JoinedRoomDetails {
    #[allow(unused)]
    room_id: OwnedRoomId,
    /// Details about the main timeline for this room.
    pub(crate) main_timeline: PerTimelineDetails,
    /// Thread-focused timelines for this room, keyed by thread root event ID.
    pub(crate) thread_timelines: HashMap<OwnedEventId, PerTimelineDetails>,
    /// The set of thread timelines currently being created, to avoid duplicate in-flight work.
    pub(crate) pending_thread_timelines: HashSet<OwnedEventId>,
    /// A drop guard for the event handler that represents a subscription to typing notices for this room.
    pub(crate) typing_notice_subscriber: Option<EventHandlerDropGuard>,
    /// A drop guard for the event handler that represents a subscription to pinned events for this room.
    pinned_events_subscriber: Option<EventHandlerDropGuard>,
}
impl Drop for JoinedRoomDetails {
    fn drop(&mut self) {
        debug!("Dropping JoinedRoomDetails for room {}", self.room_id);
        self.main_timeline.timeline_subscriber_handler_task.abort();
        for thread_timeline in self.thread_timelines.values() {
            thread_timeline.timeline_subscriber_handler_task.abort();
        }
        drop(self.typing_notice_subscriber.take());
        drop(self.pinned_events_subscriber.take());
    }
}

/// Info we store about a room received by the room list service.
///
/// This struct is necessary in order for us to track the previous state
/// of a room received from the room list service, so that we can
/// determine if the room has changed state.
/// We can't just store the `matrix_sdk::Room` object itself,
/// because that is a shallow reference to an inner room object within
/// the room list service
#[derive(Clone)]
pub struct RoomListServiceRoomInfo {
    pub room_id: OwnedRoomId,
    state: RoomState,
    is_direct: bool,
    is_marked_unread: bool,
    is_tombstoned: bool,
    tags: Option<Tags>,
    topic: Option<String>,
    user_power_levels: Option<UserPowerLevels>,
    latest_event_timestamp: Option<MilliSecondsSinceUnixEpoch>,
    num_unread_messages: u64,
    num_unread_mentions: u64,
    display_name: Option<RoomDisplayName>,
    room_avatar: Option<OwnedMxcUri>,
    heroes: Vec<RoomHero>,
    room: matrix_sdk::Room,
}

impl RoomListServiceRoomInfo {
    pub(crate) async fn from_room(
        room: matrix_sdk::Room,
        current_user_id: &Option<OwnedUserId>,
    ) -> Self {
        // Parallelize fetching of independent room data.
        let (is_direct, tags, display_name, user_power_levels) =
            tokio::join!(room.is_direct(), room.tags(), room.display_name(), async {
                if let Some(user_id) = current_user_id {
                    UserPowerLevels::from_room(&room, user_id.deref()).await
                } else {
                    None
                }
            });

        Self {
            room_id: room.room_id().to_owned(),
            state: room.state(),
            is_direct: is_direct.unwrap_or(false),
            is_marked_unread: room.is_marked_unread(),
            is_tombstoned: room.is_tombstoned(),
            tags: tags.ok().flatten(),
            topic: room.topic(),
            user_power_levels,
            latest_event_timestamp: room.latest_event_timestamp(),
            num_unread_messages: room.num_unread_messages(),
            num_unread_mentions: room.num_unread_mentions(),
            display_name: display_name.ok(),
            room_avatar: room.avatar_url(),
            heroes: room.heroes(),
            room,
        }
    }

    pub(crate) async fn from_room_ref(
        room: &matrix_sdk::Room,
        current_user_id: &Option<OwnedUserId>,
    ) -> Self {
        Self::from_room(room.clone(), current_user_id).await
    }
}

/// The number of unread messages in a room.
#[derive(Clone, Debug)]
pub enum UnreadMessageCount {
    /// There are unread messages, but we do not know how many.
    _Unknown,
    /// There are unread messages, and we know exactly how many.
    Known(u64),
}

/// Invoked when the room list service has received an update with a brand new room.
pub async fn add_new_room(
    new_room: &RoomListServiceRoomInfo,
    room_list_service: &RoomListService,
    subscribe: bool,
) -> anyhow::Result<()> {
    let direct_user_id_option = if new_room.is_direct && new_room.room.direct_targets_length() == 1
    {
        new_room
            .room
            .direct_targets()
            .iter()
            .next()
            .map(|id| id.to_owned())
    } else {
        None
    };
    match new_room.state {
        RoomState::Knocked => {
            // TODO: handle Knocked rooms (e.g., can you re-knock? or cancel a prior knock?)
            return Ok(());
        }
        RoomState::Banned => {
            info!("Got new Banned room: ({})", new_room.room_id);
            // TODO: handle rooms that this user has been banned from.
            return Ok(());
        }
        RoomState::Left => {
            info!("Got new Left room: ({:?})", new_room.room_id);
            // TODO: add this to the list of left rooms,
            //       which is collapsed by default.
            //       Upon clicking a left room, we can show a splash page
            //       that prompts the user to rejoin the room or forget it.

            // TODO: this may also be called when a user rejects an invite, not sure.
            //       So we might also need to make a new RoomsListUpdate::RoomLeft variant.
            return Ok(());
        }
        RoomState::Invited => {
            let invite_details = new_room.room.invite_details().await.ok();
            let latest_event = new_room.room.latest_event().await;
            let latest = get_latest_event_details(latest_event);

            let inviter_info = invite_details.and_then(|d| {
                d.inviter.map(|i| InviterInfo {
                    user_id: i.user_id().to_owned(),
                    display_name: i.display_name().map(|n| n.to_owned()),
                    avatar: i.avatar_url().map(|a| a.to_owned()),
                })
            });
            enqueue_rooms_list_update(RoomsListUpdate::AddInvitedRoom(InvitedRoomInfo {
                room_id: new_room.room_id.clone(),
                room_name: new_room
                    .display_name
                    .clone()
                    .unwrap_or(RoomDisplayName::Empty)
                    .into(),
                inviter_info,
                room_avatar: new_room.room_avatar.clone(),
                canonical_alias: new_room.room.canonical_alias(),
                alt_aliases: new_room.room.alt_aliases(),
                latest,
                invite_state: Default::default(),
                is_direct: new_room.is_direct,
            }));
            return Ok(());
        }
        RoomState::Joined => {} // Fall through to adding the joined room below.
    }

    // If we didn't already subscribe to this room, do so now.
    // This ensures we will properly receive all of its states and latest event.
    if subscribe {
        room_list_service
            .subscribe_to_rooms(&[&new_room.room_id])
            .await;
    }

    let timeline = Arc::new(
        new_room
            .room
            .timeline_builder()
            .with_focus(TimelineFocus::Live {
                // we show threads as separate timelines in their own RoomScreen
                hide_threaded_events: true,
            })
            .track_read_marker_and_receipts(
                matrix_sdk_ui::timeline::TimelineReadReceiptTracking::AllEvents,
            )
            .build()
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "BUG: Failed to build timeline for room {}: {e}",
                    new_room.room_id
                )
            })?,
    );

    let (timeline_update_sender, timeline_update_receiver) = crossbeam_channel::unbounded();

    let (request_sender, request_receiver) = watch::channel(Vec::new());
    let timeline_subscriber_handler_task = Handle::current().spawn(timeline_subscriber_handler(
        new_room.room.clone(),
        timeline.clone(),
        timeline_update_sender.clone(),
        request_receiver,
        None,
    ));

    let latest_event = new_room.room.latest_event().await;
    let latest = get_latest_event_details(latest_event);

    info!(
        "Adding new joined room {}, name: {:?}",
        new_room.room_id,
        new_room.room.name()
    );
    insert_room_details(
        new_room.room_id.clone(),
        JoinedRoomDetails {
            room_id: new_room.room_id.clone(),
            main_timeline: PerTimelineDetails {
                timeline,
                timeline_singleton_endpoints: Some((timeline_update_receiver, request_sender)),
                timeline_update_sender,
                timeline_subscriber_handler_task,
            },
            thread_timelines: HashMap::new(),
            pending_thread_timelines: HashSet::new(),
            typing_notice_subscriber: None,
            pinned_events_subscriber: None,
        },
    );
    // We need to add the room to the `ALL_JOINED_ROOMS` list before we can
    // send the `AddJoinedRoom` update to the UI, because the UI might immediately
    // issue a `MatrixRequest` that relies on that room being in `ALL_JOINED_ROOMS`.
    enqueue_rooms_list_update(RoomsListUpdate::AddJoinedRoom(JoinedRoomInfo {
        room_id: new_room.room_id.clone(),
        latest,
        tags: new_room.tags.clone().unwrap_or_default(),
        topic: new_room.topic.clone(),
        num_unread_messages: new_room.num_unread_messages,
        num_unread_mentions: new_room.num_unread_mentions,
        is_marked_unread: new_room.is_marked_unread,
        avatar: new_room.room_avatar.clone(),
        room_name: new_room
            .display_name
            .clone()
            .unwrap_or(RoomDisplayName::Empty)
            .into(),
        canonical_alias: new_room.room.canonical_alias(),
        alt_aliases: new_room.room.alt_aliases(),
        has_been_paginated: false,
        is_selected: false,
        is_direct: new_room.is_direct,
        is_tombstoned: new_room.is_tombstoned,
        direct_user_id: direct_user_id_option.and_then(|id| id.into_user_id()),
        heroes: new_room.heroes.clone(),
    }));
    Ok(())
}

/// Invoked when the room list service has received an update that changes an existing room.
pub async fn update_room(
    old_room: &RoomListServiceRoomInfo,
    new_room: &RoomListServiceRoomInfo,
    room_list_service: &RoomListService,
) -> anyhow::Result<()> {
    let new_room_id = new_room.room_id.clone();
    if old_room.room_id == new_room_id {
        // Handle state transitions for a room.
        info!(
            "Room {:?} ({new_room_id}) state went from {:?} --> {:?}",
            new_room.display_name, old_room.state, new_room.state
        );
        if old_room.state != new_room.state {
            match new_room.state {
                RoomState::Banned => {
                    // TODO: handle rooms that this user has been banned from.
                    debug!(
                        "Removing Banned room: {:?} ({new_room_id})",
                        new_room.display_name
                    );
                    remove_room(new_room);
                    return Ok(());
                }
                RoomState::Left => {
                    debug!(
                        "Removing Left room: {:?} ({new_room_id})",
                        new_room.display_name
                    );
                    // TODO: instead of removing this, we could optionally add it to
                    //       a separate list of left rooms, which would be collapsed by default.
                    //       Upon clicking a left room, we could show a splash page
                    //       that prompts the user to rejoin the room or forget it permanently.
                    //       Currently, we just remove it and do not show left rooms at all.
                    remove_room(new_room);
                    return Ok(());
                }
                RoomState::Joined => {
                    debug!(
                        "update_room(): adding new Joined room: {:?} ({new_room_id})",
                        new_room.display_name
                    );
                    return add_new_room(new_room, room_list_service, true).await;
                }
                RoomState::Invited => {
                    debug!(
                        "update_room(): adding new Invited room: {:?} ({new_room_id})",
                        new_room.display_name
                    );
                    return add_new_room(new_room, room_list_service, true).await;
                }
                RoomState::Knocked => {
                    // TODO: handle Knocked rooms (e.g., can you re-knock? or cancel a prior knock?)
                    return Ok(());
                }
            }
        }

        // First, we check for changes to room data that is relevant to any room,
        // including joined, invited, and other rooms.
        // This includes the room name and room avatar.
        if old_room.room_avatar != new_room.room_avatar
            && let Some(ref avatar) = new_room.room_avatar
        {
            debug!("Updating room avatar for room {}", new_room_id);
            enqueue_rooms_list_update(RoomsListUpdate::UpdateRoomAvatar {
                room_id: new_room_id.clone(),
                avatar: avatar.to_owned(),
            });
        }
        if old_room.display_name != new_room.display_name
            && let Some(ref new_room_name) = new_room.display_name
        {
            debug!(
                "Updating room {} name: {:?} --> {:?}",
                new_room_id, old_room.display_name, new_room_name
            );
            enqueue_rooms_list_update(RoomsListUpdate::UpdateRoomName {
                room_id: new_room_id.clone(),
                new_room_name: new_room_name.to_owned(),
            });
        }

        if old_room.topic != new_room.topic
            && let Some(ref new_topic) = new_room.topic
        {
            debug!(
                "Updating room {} topic: {:?} --> {:?}",
                new_room_id, old_room.topic, new_topic
            );
            enqueue_rooms_list_update(RoomsListUpdate::UpdateTopic {
                room_id: new_room_id.clone(),
                new_topic: new_topic.to_owned(),
            });
        }

        // Then, we check for changes to room data that is only relevant to joined rooms:
        // including the latest event, tags, unread counts, is_direct, tombstoned state, power levels, etc.
        // Invited or left rooms don't care about these details.
        if matches!(new_room.state, RoomState::Joined) {
            // For some reason, the latest event API does not reliably catch *all* changes
            // to the latest event in a given room, such as redactions.
            // Thus, we have to re-obtain the latest event on *every* update, regardless of timestamp.
            //
            let update_latest = match (
                old_room.latest_event_timestamp,
                new_room.room.latest_event_timestamp(),
            ) {
                (Some(old_ts), Some(new_ts)) => new_ts >= old_ts,
                (None, Some(_)) => true,
                _ => false,
            };
            if update_latest {
                update_latest_event(&new_room.room).await;
            }

            if old_room.tags != new_room.tags {
                debug!(
                    "Updating room {} tags from {:?} to {:?}",
                    new_room_id, old_room.tags, new_room.tags
                );
                enqueue_rooms_list_update(RoomsListUpdate::Tags {
                    room_id: new_room_id.clone(),
                    new_tags: new_room.tags.clone().unwrap_or_default(),
                });
            }

            if old_room.is_marked_unread != new_room.is_marked_unread
                || old_room.num_unread_messages != new_room.num_unread_messages
                || old_room.num_unread_mentions != new_room.num_unread_mentions
            {
                debug!(
                    "Updating room {}, unread messages {} --> {}, unread mentions {} --> {}",
                    new_room_id,
                    old_room.num_unread_messages,
                    new_room.num_unread_messages,
                    old_room.num_unread_mentions,
                    new_room.num_unread_mentions,
                );
                enqueue_rooms_list_update(RoomsListUpdate::UpdateNumUnreadMessages {
                    room_id: new_room_id.clone(),
                    is_marked_unread: new_room.is_marked_unread,
                    unread_messages: UnreadMessageCount::Known(new_room.num_unread_messages),
                    unread_mentions: new_room.num_unread_mentions,
                });
            }

            if old_room.is_direct != new_room.is_direct {
                debug!(
                    "Updating room {} is_direct from {} to {}",
                    new_room_id, old_room.is_direct, new_room.is_direct,
                );
                enqueue_rooms_list_update(RoomsListUpdate::UpdateIsDirect {
                    room_id: new_room_id.clone(),
                    is_direct: new_room.is_direct,
                });
            }

            let mut __timeline_update_sender_opt = None;
            let mut get_timeline_update_sender = |room_id| {
                if __timeline_update_sender_opt.is_none()
                    && let Some(jrd) = try_get_room_details(room_id)
                {
                    __timeline_update_sender_opt = Some(
                        jrd.lock()
                            .unwrap()
                            .main_timeline
                            .timeline_update_sender
                            .clone(),
                    );
                }
                __timeline_update_sender_opt.clone()
            };

            if !old_room.is_tombstoned && new_room.is_tombstoned {
                let successor_room = new_room.room.successor_room();
                debug!("Updating room {new_room_id} to be tombstoned, {successor_room:?}");
                enqueue_rooms_list_update(RoomsListUpdate::TombstonedRoom {
                    room_id: new_room_id.clone(),
                });
                if get_timeline_update_sender(&new_room_id).is_some() {
                    // TODO: implement this from robrix
                    // spawn_fetch_successor_room_preview(
                    //     room_list_service.client().clone(),
                    //     successor_room,
                    //     new_room_id.clone(),
                    //     timeline_update_sender,
                    // );
                } else {
                    error!(
                        "BUG: could not find JoinedRoomDetails for newly-tombstoned room {new_room_id}"
                    );
                }
            }

            if let Some(nupl) = new_room.user_power_levels
                && old_room.user_power_levels.is_none_or(|oupl| oupl != nupl)
            {
                if let Some(timeline_update_sender) = get_timeline_update_sender(&new_room_id) {
                    debug!("Updating room {new_room_id} user power levels.");
                    match timeline_update_sender.send(TimelineUpdate::UserPowerLevels(nupl)) {
                        Ok(_) => {
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        }
                        Err(_) => {
                            warn!("Failed to send the UserPowerLevels update to room {new_room_id}")
                        }
                    }
                } else {
                    error!(
                        "BUG: could not find JoinedRoomDetails for room {new_room_id} where power levels changed."
                    );
                }
            }
        }
        Ok(())
    } else {
        debug!(
            "UNTESTED SCENARIO: update_room(): removing old room {}, replacing with new room {}",
            old_room.room_id, new_room_id,
        );
        remove_room(old_room);
        add_new_room(new_room, room_list_service, true).await
    }
}

/// Invoked when the room list service has received an update to remove an existing room.
pub fn remove_room(room: &RoomListServiceRoomInfo) {
    remove_room_details(&room.room_id);
    enqueue_rooms_list_update(RoomsListUpdate::RemoveRoom {
        room_id: room.room_id.clone(),
        _new_state: room.state,
    });
}

//
// The actual singleton that holds the joined rooms

type JoinedRoomsMap = BTreeMap<OwnedRoomId, Arc<Mutex<JoinedRoomDetails>>>;

/// Information about all joined rooms that our client currently know about.
pub static ALL_JOINED_ROOMS: (Mutex<JoinedRoomsMap>, Condvar) =
    (Mutex::new(BTreeMap::new()), Condvar::new());

/// Wait for joined room details that we know it is going to be initialized.
/// Do not use this method if you're not sure the room will be init.
pub fn _wait_for_room_details(room_id: &OwnedRoomId) -> Arc<Mutex<JoinedRoomDetails>> {
    let (lock, cvar) = &ALL_JOINED_ROOMS;

    let mut map_guard = lock.lock().unwrap();

    // Wait until the key is present
    while map_guard.get(room_id).is_none() {
        map_guard = cvar.wait(map_guard).unwrap();
    }

    // map_guard is dropped, releasing the Mutex lock.
    map_guard.get(room_id).unwrap().clone()
}

/// Inserts new room details and notifies any waiting threads.
pub fn insert_room_details(room_id: OwnedRoomId, details: JoinedRoomDetails) {
    let (lock, cvar) = &ALL_JOINED_ROOMS;

    let details_arc_mutex = Arc::new(Mutex::new(details));

    let mut map_guard = lock.lock().unwrap();
    map_guard.insert(room_id, details_arc_mutex);

    // Notify the waiters
    cvar.notify_all();

    // map_guard drops, releasing the BTreeMap lock.
}

/// Removes a room's details from the map and returns the Arc<Mutex<T>> if present.
pub fn remove_room_details(room_id: &OwnedRoomId) -> Option<Arc<Mutex<JoinedRoomDetails>>> {
    let (lock, cvar) = &ALL_JOINED_ROOMS;

    let mut map_guard = lock.lock().unwrap();

    let removed_details = map_guard.remove(room_id);

    // Notify the waiters (if any threads were waiting on a state change).
    cvar.notify_all();

    // map_guard drops, releasing the BTreeMap lock.
    removed_details
}

/// Tries to get the value for the given room ID immediately.
pub fn try_get_room_details(room_id: &OwnedRoomId) -> Option<Arc<Mutex<JoinedRoomDetails>>> {
    let (lock, _cvar) = &ALL_JOINED_ROOMS;

    let map_guard = lock.lock().unwrap();

    // map_guard drops, releasing the BTreeMap lock.
    map_guard.get(room_id).cloned()
}

/// Clears all entries from the map.
/// This method acquires the outer lock, clears the BTreeMap, and notifies all waiters.
pub fn clear_all_rooms() {
    let (lock, cvar) = &ALL_JOINED_ROOMS;

    let mut map_guard = lock.lock().unwrap();

    // Clear the map. This drops all contained Arc<Mutex<JoinedRoomDetails>>.
    // The reference count of each inner Mutex<JoinedRoomDetails> is reduced.
    // If the count hits zero, the underlying details are dropped.
    map_guard.clear();

    // Notify the waiters.
    cvar.notify_all();

    // map_guard is dropped here, releasing the BTreeMap lock.
}

fn with_per_timeline_details<F, R>(kind: &TimelineKind, f: F) -> Option<R>
where
    F: FnOnce(&mut PerTimelineDetails) -> R,
{
    let lock = crate::room::joined_room::try_get_room_details(kind.room_id())?;
    let mut room_info = lock.lock().unwrap();

    let details = match kind {
        TimelineKind::MainRoom { .. } => Some(&mut room_info.main_timeline),
        TimelineKind::Thread {
            thread_root_event_id,
            ..
        } => room_info.thread_timelines.get_mut(thread_root_event_id),
    }?;

    Some(f(details))
}

/// Obtains the lock on `ALL_JOINED_ROOMS` and returns the timeline and timeline update sender for the given timeline kind.
pub(crate) fn get_timeline_and_sender(
    kind: &TimelineKind,
) -> Option<(Arc<Timeline>, crossbeam_channel::Sender<TimelineUpdate>)> {
    with_per_timeline_details(kind, |details| {
        (
            details.timeline.clone(),
            details.timeline_update_sender.clone(),
        )
    })
}

/// Obtains the lock on `ALL_JOINED_ROOMS` and returns the timeline for the given timeline kind.
pub(crate) fn get_timeline(kind: &TimelineKind) -> Option<Arc<Timeline>> {
    with_per_timeline_details(kind, |details| details.timeline.clone())
}
