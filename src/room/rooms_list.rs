use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    sync::Arc,
};
use tracing::{debug, error, info, warn};
use ts_rs::TS;

use crossbeam_queue::SegQueue;
use eyeball::Subscriber;
use matrix_sdk::{
    RoomDisplayName, RoomHero, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, OwnedUserId,
        events::tag::Tags,
    },
};
use matrix_sdk_ui::room_list_service::RoomListLoadingState;
use serde::Serialize;
use tokio::{
    runtime::Handle,
    sync::oneshot::{self, Sender},
};

use crate::{
    init::singletons::{ALL_ROOMS_LOADED, UIUpdateMessage, broadcast_event},
    models::{
        events::{ToastNotificationRequest, ToastNotificationVariant},
        room_display_name::FrontendRoomDisplayName,
        state_updater::StateUpdater,
    },
    room::{
        invited_room::InvitedRoomInfo,
        joined_room::UnreadMessageCount,
        notifications::enqueue_toast_notification,
        room_filter::{RoomDisplayFilterBuilder, RoomFilterCriteria, SortFn},
    },
    stores::room_store::send_room_creation_request_and_await_response,
};

use super::{room_filter::RoomDisplayFilter, room_screen::RoomScreen};

/// The possible updates that should be displayed by the single list of all rooms.
///
/// These updates are enqueued by the `enqueue_rooms_list_update` function
/// (which is called from background async tasks that receive updates from the matrix server),
/// and then dequeued by the `RoomsList` widget's `handle_event` function.
#[derive(Debug)]
pub enum RoomsListUpdate {
    /// No rooms have been loaded yet.
    NotLoaded,
    /// Some rooms were loaded, and the server optionally told us
    /// the max number of rooms that will ever be loaded.
    LoadedRooms { max_rooms: Option<u32> },
    /// Add a new room to the list of rooms the user has been invited to.
    /// This will be maintained and displayed separately from joined rooms.
    AddInvitedRoom(InvitedRoomInfo),
    /// Add a new room to the list of all rooms that the user has joined.
    AddJoinedRoom(JoinedRoomInfo),
    /// Clear all rooms in the list of all rooms.
    ClearRooms,
    /// Update the latest event content and timestamp for the given room.
    UpdateLatestEvent {
        room_id: OwnedRoomId,
        timestamp: MilliSecondsSinceUnixEpoch,
        /// The Html-formatted text preview of the latest message.
        latest_message_text: String,
    },
    /// Update the number of unread messages for the given room.
    UpdateNumUnreadMessages {
        room_id: OwnedRoomId,
        unread_messages: UnreadMessageCount,
        unread_mentions: u64,
    },
    /// Update the displayable name for the given room.
    UpdateRoomName {
        room_id: OwnedRoomId,
        new_room_name: RoomDisplayName,
    },
    /// Update the topic for the given room.
    UpdateTopic {
        room_id: OwnedRoomId,
        new_topic: String,
    },
    /// Update the avatar (image) for the given room.
    UpdateRoomAvatar {
        room_id: OwnedRoomId,
        avatar: OwnedMxcUri,
    },
    /// Update whether the given room is a direct room.
    UpdateIsDirect {
        room_id: OwnedRoomId,
        is_direct: bool,
    },
    /// Remove the given room from the rooms list
    RemoveRoom {
        room_id: OwnedRoomId,
        /// The new state of the room (which caused its removal).
        _new_state: RoomState,
    },
    /// Update the tags for the given room.
    Tags {
        room_id: OwnedRoomId,
        new_tags: Tags,
    },
    /// Update the status label at the bottom of the list of all rooms.
    Status { status: RoomsCollectionStatus },
    /// Mark the given room as tombstoned.
    TombstonedRoom { room_id: OwnedRoomId },
    /// Apply a filter to the rooms list
    ApplyFilter { keywords: String },
}

static PENDING_ROOM_UPDATES: SegQueue<RoomsListUpdate> = SegQueue::new();

/// Enqueue a new room update for the list of all rooms
/// and signals the UI that a new update is available to be handled.
pub fn enqueue_rooms_list_update(update: RoomsListUpdate) {
    PENDING_ROOM_UPDATES.push(update);
    broadcast_event(UIUpdateMessage::RefreshUI);
}

/// UI-related info about a joined room.
///
/// This includes info needed display a preview of that room in the RoomsList
/// and to filter the list of rooms based on the current search filter.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JoinedRoomInfo {
    /// The matrix ID of this room.
    pub(crate) room_id: OwnedRoomId,
    /// The displayable name of this room, if known.
    pub(crate) room_name: FrontendRoomDisplayName,
    /// The number of unread messages in this room.
    pub(crate) num_unread_messages: u64,
    /// The number of unread mentions in this room.
    pub(crate) num_unread_mentions: u64,
    /// The canonical alias for this room, if any.
    pub(crate) canonical_alias: Option<OwnedRoomAliasId>,
    /// The alternative aliases for this room, if any.
    pub(crate) alt_aliases: Vec<OwnedRoomAliasId>,
    /// The tags associated with this room, if any.
    /// This includes things like is_favourite, is_low_priority,
    /// whether the room is a server notice room, etc.
    pub(crate) tags: Tags,
    /// The topic of the current room
    pub(crate) topic: Option<String>,
    /// The timestamp and Html text content of the latest message in this room.
    pub(crate) latest: Option<(MilliSecondsSinceUnixEpoch, String)>,
    /// The avatar for this room
    pub(crate) avatar: Option<OwnedMxcUri>,
    /// Whether this room has been paginated at least once.
    /// We pre-paginate visible rooms at least once in order to
    /// be able to display the latest message in the room preview,
    /// and to have something to immediately show when a user first opens a room.
    pub(crate) has_been_paginated: bool,
    /// Whether this room is currently selected in the UI.
    pub(crate) is_selected: bool,
    /// Whether this a DM room or not.
    pub(crate) is_direct: bool,
    /// UserId of the user if the room is direct
    pub(crate) direct_user_id: Option<OwnedUserId>,
    /// Whether this room is tombstoned (shut down and replaced with a successor room).
    pub(crate) is_tombstoned: bool,
    /// Room "heroes", ~ main users of this room
    pub(crate) heroes: Vec<RoomHero>,
}

pub fn handle_rooms_loading_state(mut loading_state: Subscriber<RoomListLoadingState>) {
    debug!(
        "Initial room list loading state is {:?}",
        loading_state.get()
    );
    Handle::current().spawn(async move {
        while let Some(state) = loading_state.next().await {
            debug!("Received a room list loading state update: {state:?}");
            match state {
                RoomListLoadingState::NotLoaded => {
                    enqueue_rooms_list_update(RoomsListUpdate::NotLoaded);
                }
                RoomListLoadingState::Loaded {
                    maximum_number_of_rooms,
                } => {
                    ALL_ROOMS_LOADED.get_or_init(|| true);
                    enqueue_rooms_list_update(RoomsListUpdate::LoadedRooms {
                        max_rooms: maximum_number_of_rooms,
                    });
                }
            }
        }
    });
}

/// The struct containing all the data related to the homepage rooms list.
/// Fields are not exposed to the adapter directly, the adapter can only serialize this struct.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RoomsList {
    // We manually put the Record type, otherwise it generates a weird type with undefined
    #[ts(type = "Record<string, InvitedRoomInfo>")]
    /// The list of all rooms that the user has been invited to.
    invited_rooms: HashMap<OwnedRoomId, InvitedRoomInfo>,

    // We manually put the Record type, otherwise it generates a weird type with undefined
    #[ts(type = "Record<string, JoinedRoomInfo>")]
    /// The set of all joined rooms and their cached preview info.
    all_joined_rooms: HashMap<OwnedRoomId, JoinedRoomInfo>,

    /// The currently-active filter function for the list of rooms.
    ///
    /// Note: for performance reasons, this does not get automatically applied
    /// when its value changes. Instead, you must manually invoke it on the set of `all_joined_rooms`
    /// in order to update the set of `displayed_rooms` accordingly.
    #[serde(skip)]
    display_filter: RoomDisplayFilter,

    /// The latest keywords entered into the `RoomFilterInputBar`.
    ///
    /// If empty, there are no filter keywords in use, and all rooms/spaces should be shown.
    filter_keywords: String,

    /// The list of invited rooms currently displayed in the UI, in order from top to bottom.
    /// This is a strict subset of the rooms present in `all_invited_rooms`, and should be determined
    /// by applying the `display_filter` to the set of `all_invited_rooms`.
    displayed_invited_rooms: Vec<OwnedRoomId>,

    /// The list of direct rooms currently displayed in the UI, in order from top to bottom.
    /// This is a strict subset of the rooms present in `all_joined_rooms`,
    /// and should be determined by applying the `display_filter && is_direct`
    /// to the set of `all_joined_rooms`.
    displayed_direct_rooms: Vec<OwnedRoomId>,

    /// The list of regular (non-direct) joined rooms currently displayed in the UI,
    /// in order from top to bottom.
    /// This is a strict subset of the rooms in `all_joined_rooms`,
    /// and should be determined by applying the `display_filter && !is_direct`
    /// to the set of `all_joined_rooms`.
    ///
    /// **Direct rooms are excluded** from this; they are in `displayed_direct_rooms`.
    displayed_regular_rooms: Vec<OwnedRoomId>,

    /// The latest status message that should be displayed in the bottom status label.
    status: RoomsCollectionStatus,
    /// The ID of the currently-selected room.
    current_active_room: Option<OwnedRoomId>,
    /// The current active room sender to interrupt the task when room is closed.
    /// Backend only
    #[serde(skip)]
    current_active_room_killer: Option<Sender<()>>,
    /// The maximum number of rooms that will ever be loaded.
    max_known_rooms: Option<u32>,
    /// The state updater passed by the adapter for this struct
    #[serde(skip)]
    state_updaters: Arc<Box<dyn StateUpdater>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "status",
    content = "message"
)]
pub enum RoomsCollectionStatus {
    NotLoaded(String),
    Loading(String),
    Loaded(String),
    Error(String),
}

impl RoomsList {
    pub(crate) fn new(updaters: Arc<Box<dyn StateUpdater>>) -> Self {
        Self {
            invited_rooms: HashMap::default(),
            all_joined_rooms: HashMap::default(),
            display_filter: RoomDisplayFilter::default(),
            filter_keywords: "".to_owned(),
            displayed_regular_rooms: Vec::new(),
            displayed_direct_rooms: Vec::new(),
            displayed_invited_rooms: Vec::new(),
            status: RoomsCollectionStatus::NotLoaded("Initiating".to_owned()),
            current_active_room: None,
            current_active_room_killer: None,
            max_known_rooms: None,
            state_updaters: updaters,
        }
    }

    fn update_frontend_state(&self) {
        if let Err(e) = self.state_updaters.update_rooms_list(self) {
            enqueue_toast_notification(ToastNotificationRequest::new(
                format!("Cannot update room list store. Error: {e}"),
                None,
                ToastNotificationVariant::Error,
            ))
        }
    }

    /// Handle all pending updates to the list of all rooms.
    pub(crate) async fn handle_rooms_list_updates(&mut self) {
        let mut num_updates: usize = 0;
        while let Some(update) = PENDING_ROOM_UPDATES.pop() {
            num_updates += 1;

            debug!("Processing update type: {update:?}");

            match update {
                RoomsListUpdate::AddInvitedRoom(invited_room) => {
                    let room_id = invited_room.room_id.clone();
                    let should_display = (self.display_filter)(&invited_room);
                    let _replaced = self
                        .invited_rooms
                        .borrow_mut()
                        .insert(room_id.clone(), invited_room);
                    if let Some(_old_room) = _replaced {
                        error!("BUG: Added invited room {room_id} that already existed");
                    } else if should_display {
                        self.displayed_invited_rooms.push(room_id);
                    }
                    self.update_status_rooms_count();
                }
                RoomsListUpdate::AddJoinedRoom(joined_room) => {
                    let room_id = joined_room.room_id.clone();
                    let should_display = (self.display_filter)(&joined_room);
                    let is_direct = joined_room.is_direct;

                    let replaced = self.all_joined_rooms.insert(room_id.clone(), joined_room);

                    if let Some(_old_room) = replaced {
                        error!("BUG: Added joined room {room_id} that already existed");
                    } else if should_display {
                        // Create frontend state store
                        if let Err(e) =
                            send_room_creation_request_and_await_response(room_id.as_str()).await
                        {
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!(
                                    "Cannot create frontend store for room {}. Error: {e}",
                                    room_id.as_str()
                                ),
                                None,
                                ToastNotificationVariant::Error,
                            ))
                        }
                        if is_direct {
                            self.displayed_direct_rooms.push(room_id.clone());
                        } else {
                            self.displayed_regular_rooms.push(room_id.clone());
                        }
                    }
                    // If this room was added as a result of accepting an invite, we must:
                    // 1. Remove the room from the list of invited rooms.
                    // 2. Update the displayed invited rooms list to remove this room.
                    if let Some(_accepted_invite) = self.invited_rooms.borrow_mut().remove(&room_id)
                    {
                        info!("Removed room {room_id} from the list of invited rooms");
                        self.displayed_invited_rooms
                            .iter()
                            .position(|r| r == &room_id)
                            .map(|index| self.displayed_invited_rooms.remove(index));
                    }
                    self.update_status_rooms_count();
                }
                RoomsListUpdate::UpdateRoomAvatar { room_id, avatar } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        room.avatar = Some(avatar.clone());
                    } else {
                        error!("Error: couldn't find room {room_id} to update avatar");
                    }
                }
                RoomsListUpdate::UpdateLatestEvent {
                    room_id,
                    timestamp,
                    latest_message_text,
                } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        room.latest = Some((timestamp, latest_message_text.clone()));
                    } else {
                        warn!("Error: couldn't find room {room_id} to update latest event");
                    }
                }
                RoomsListUpdate::UpdateNumUnreadMessages {
                    room_id,
                    unread_messages,
                    unread_mentions,
                } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        (room.num_unread_messages, room.num_unread_mentions) = match unread_messages
                        {
                            UnreadMessageCount::_Unknown => (0, 0),
                            UnreadMessageCount::Known(count) => (count, unread_mentions),
                        };
                    } else {
                        warn!(
                            "Warning: couldn't find room {} to update unread messages count",
                            room_id
                        );
                    }
                }
                RoomsListUpdate::UpdateRoomName {
                    room_id,
                    new_room_name,
                } => {
                    // Try to update joined room first
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        let was_displayed = (self.display_filter)(room);
                        // Update with the new RoomName (preserves EmptyWas semantics)
                        room.room_name = new_room_name.into();
                        let should_display = (self.display_filter)(room);
                        match (was_displayed, should_display) {
                            // No need to update the displayed rooms list.
                            (true, true) | (false, false) => {}
                            // Room was displayed but should no longer be displayed.
                            (true, false) => {
                                if room.is_direct {
                                    self.displayed_direct_rooms
                                        .iter()
                                        .position(|r| r == &room_id)
                                        .map(|index| self.displayed_direct_rooms.remove(index));
                                } else {
                                    self.displayed_regular_rooms
                                        .iter()
                                        .position(|r| r == &room_id)
                                        .map(|index| self.displayed_regular_rooms.remove(index));
                                }
                            }
                            // Room was not displayed but should now be displayed.
                            (false, true) => {
                                if room.is_direct {
                                    self.displayed_direct_rooms.push(room_id);
                                } else {
                                    self.displayed_regular_rooms.push(room_id);
                                }
                            }
                        }
                    }
                    // If not a joined room, try to update invited room
                    else {
                        let invited_rooms = self.invited_rooms.borrow_mut();
                        if let Some(invited_room) = invited_rooms.get_mut(&room_id) {
                            let was_displayed = (self.display_filter)(invited_room);
                            invited_room.room_name = new_room_name.into();
                            let should_display = (self.display_filter)(invited_room);
                            match (was_displayed, should_display) {
                                (true, true) | (false, false) => {}
                                (true, false) => {
                                    self.displayed_invited_rooms
                                        .iter()
                                        .position(|r| r == &room_id)
                                        .map(|index| self.displayed_invited_rooms.remove(index));
                                }
                                (false, true) => {
                                    self.displayed_invited_rooms.push(room_id.clone());
                                }
                            }
                        } else {
                            warn!(
                                "Warning: couldn't find invited room {} to update room name",
                                room_id
                            );
                        }
                    }
                }
                RoomsListUpdate::UpdateTopic { room_id, new_topic } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        room.topic = Some(new_topic);
                    }
                }
                RoomsListUpdate::UpdateIsDirect { room_id, is_direct } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        if room.is_direct == is_direct {
                            continue;
                        }
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            format!(
                                "{} was changed from {} to {}.",
                                room.room_id,
                                if room.is_direct { "direct" } else { "regular" },
                                if is_direct { "direct" } else { "regular" }
                            ),
                            None,
                            ToastNotificationVariant::Info,
                        ));
                        // If the room was currently displayed, remove it from the proper list.
                        if (self.display_filter)(room) {
                            let list_to_remove_from = if room.is_direct {
                                &mut self.displayed_direct_rooms
                            } else {
                                &mut self.displayed_regular_rooms
                            };
                            list_to_remove_from
                                .iter()
                                .position(|r| r == &room_id)
                                .map(|index| list_to_remove_from.remove(index));
                        }
                        // Update the room. If it should now be displayed, add it to the correct list.
                        room.is_direct = is_direct;
                        if (self.display_filter)(room) {
                            if is_direct {
                                self.displayed_direct_rooms.push(room_id);
                            } else {
                                self.displayed_regular_rooms.push(room_id);
                            }
                        }
                    } else {
                        error!("Error: couldn't find room {room_id} to update is_direct");
                    }
                }
                RoomsListUpdate::RemoveRoom {
                    room_id,
                    _new_state: _,
                } => {
                    if let Some(removed) = self.all_joined_rooms.remove(&room_id) {
                        info!("Removed room {room_id} from the list of all joined rooms");
                        if removed.is_direct {
                            self.displayed_direct_rooms
                                .iter()
                                .position(|r| r == &room_id)
                                .map(|index| self.displayed_direct_rooms.remove(index));
                        } else {
                            self.displayed_regular_rooms
                                .iter()
                                .position(|r| r == &room_id)
                                .map(|index| self.displayed_regular_rooms.remove(index));
                        }
                    } else if let Some(_removed) = self.invited_rooms.borrow_mut().remove(&room_id)
                    {
                        info!("Removed room {room_id} from the list of all invited rooms");
                        self.displayed_invited_rooms
                            .iter()
                            .position(|r| r == &room_id)
                            .map(|index| self.displayed_invited_rooms.remove(index));
                    }

                    self.update_status_rooms_count();
                }
                RoomsListUpdate::ClearRooms => {
                    self.all_joined_rooms.clear();
                    self.displayed_direct_rooms.clear();
                    self.displayed_regular_rooms.clear();
                    self.invited_rooms.borrow_mut().clear();
                    self.displayed_invited_rooms.clear();
                    self.update_status_rooms_count();
                }
                RoomsListUpdate::NotLoaded => {
                    self.status = RoomsCollectionStatus::Loading(
                        "Loading rooms (waiting for homeserver)...".to_owned(),
                    );
                }
                RoomsListUpdate::LoadedRooms { max_rooms } => {
                    self.max_known_rooms = max_rooms;
                    self.update_status_rooms_count();
                }
                RoomsListUpdate::Tags { room_id, new_tags } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        room.tags = new_tags;
                    } else if let Some(_room) = self.invited_rooms.borrow().get(&room_id) {
                        debug!("Ignoring updated tags update for invited room {room_id}");
                    } else {
                        warn!("Error: skipping updated Tags for unknown room {room_id}.");
                    }
                }
                RoomsListUpdate::Status { status } => {
                    self.status = status;
                }
                RoomsListUpdate::TombstonedRoom { room_id } => {
                    if let Some(room) = self.all_joined_rooms.get_mut(&room_id) {
                        let was_displayed = (self.display_filter)(room);
                        room.is_tombstoned = true;
                        let should_display = (self.display_filter)(room);
                        match (was_displayed, should_display) {
                            // No need to update the displayed rooms list.
                            (true, true) | (false, false) => {}
                            // Room was displayed but should no longer be displayed.
                            (true, false) => {
                                if room.is_direct {
                                    self.displayed_direct_rooms
                                        .iter()
                                        .position(|r| r == &room_id)
                                        .map(|index| self.displayed_direct_rooms.remove(index));
                                } else {
                                    self.displayed_regular_rooms
                                        .iter()
                                        .position(|r| r == &room_id)
                                        .map(|index| self.displayed_regular_rooms.remove(index));
                                }
                            }
                            // Room was not displayed but should now be displayed.
                            (false, true) => {
                                if room.is_direct {
                                    self.displayed_direct_rooms.push(room_id);
                                } else {
                                    self.displayed_regular_rooms.push(room_id);
                                }
                            }
                        }
                    } else {
                        warn!(
                            "Warning: couldn't find room {room_id} to update the tombstone status"
                        );
                    }
                }
                RoomsListUpdate::ApplyFilter { keywords } => {
                    self.filter_keywords = keywords;
                    // The filter will be applied at the end
                }
            }
        }
        if num_updates > 0 {
            debug!(
                "RoomsList: processed {} updates to the list of all rooms",
                num_updates
            );
            self.update_displayed_rooms();
            self.update_frontend_state();
        }
    }

    pub(crate) fn handle_current_active_room(
        &mut self,
        updated_current_active_room: OwnedRoomId,
        room_name: String,
    ) -> anyhow::Result<()> {
        // Don't do anything if the room is already active
        if self
            .current_active_room
            .as_ref()
            .is_some_and(|id| id == &updated_current_active_room)
        {
            return Ok(());
        } else if let Some(sender) = self.current_active_room_killer.take() {
            sender
                .send(())
                .expect("Error while sending message to terminate RoomScreen thread {e:?}")
        } else {
            debug!("First time setting a room");
        }

        debug!("Current active room: {updated_current_active_room:?}");

        self.current_active_room = Some(updated_current_active_room.clone());

        let mut ui_subscriber = crate::init::singletons::subscribe_to_events()
            .expect("Couldn't get UI subscriber event");
        let (tx, mut rx) = oneshot::channel::<()>();
        self.current_active_room_killer = Some(tx);
        let updaters = self.state_updaters.clone();
        Handle::current().spawn(async move {
            let mut room_screen = RoomScreen::new(updaters, updated_current_active_room, room_name);
            room_screen.show_timeline();

            loop {
                tokio::select! {
                    _ = ui_subscriber.recv() => {
                        room_screen.process_timeline_updates();
                    }
                    _ = &mut rx => {
                        break;
                    }
                }
            }
            // as soon as this task is done,
            // the room_screen will be dropped,
            // and hide_timeline() will be called on drop
        });
        Ok(())

        // This function closes the previous room thread and opens a new.
        // this thread will also handle room actions such as sending messages
    }

    /// Updates the status message to show how many rooms have been loaded.
    fn update_status_rooms_count(&mut self) {
        let num_rooms = self.all_joined_rooms.len() + self.invited_rooms.borrow().len();
        self.status = if let Some(max_rooms) = self.max_known_rooms {
            let message = format!("Loaded {num_rooms} of {max_rooms} total rooms.");
            if num_rooms as u32 == max_rooms {
                RoomsCollectionStatus::Loaded(message)
            } else {
                RoomsCollectionStatus::Loading(message)
            }
        } else {
            RoomsCollectionStatus::Loaded(format!("Loaded {num_rooms} rooms."))
        };
    }

    /// Updates the status message to show how many rooms are currently displayed
    /// that match the current search filter.
    fn set_status_to_matching_rooms(&mut self) {
        let num_rooms = self.displayed_invited_rooms.len()
            + self.displayed_direct_rooms.len()
            + self.displayed_regular_rooms.len();
        self.status = match num_rooms {
            0 => RoomsCollectionStatus::Loaded("No matching rooms found.".to_owned()),
            1 => RoomsCollectionStatus::Loaded("Found 1 matching room.".to_owned()),
            n => RoomsCollectionStatus::Loaded(format!("Found {} matching rooms.", n)),
        }
    }

    /// Updates the lists of displayed rooms based on the current search filter
    /// and redraws the RoomsList.
    fn update_displayed_rooms(&mut self) {
        let (filter, sort_fn) = if self.filter_keywords.is_empty() {
            RoomDisplayFilterBuilder::default()
                .sort_by_latest_ts()
                .build()
        } else {
            RoomDisplayFilterBuilder::new()
                .set_keywords(self.filter_keywords.clone())
                .set_filter_criteria(RoomFilterCriteria::All)
                .sort_by_latest_ts()
                .build()
        };
        self.display_filter = filter;

        self.displayed_invited_rooms = self.generate_displayed_invited_rooms(sort_fn.as_deref());

        let (new_displayed_regular_rooms, new_displayed_direct_rooms) =
            self.generate_displayed_joined_rooms(sort_fn.as_deref());

        self.displayed_regular_rooms = new_displayed_regular_rooms;
        self.displayed_direct_rooms = new_displayed_direct_rooms;

        if self.filter_keywords.is_empty() {
            self.update_status_rooms_count();
        } else {
            self.set_status_to_matching_rooms();
        }
    }

    /// Generates the list of displayed invited rooms based on the current filter
    /// and the given sort function.
    fn generate_displayed_invited_rooms(&self, sort_fn: Option<&SortFn>) -> Vec<OwnedRoomId> {
        let invited_rooms_ref = self.invited_rooms.borrow();
        let filtered_invited_rooms_iter = invited_rooms_ref
            .iter()
            .filter(|(_room_id, room)| (self.display_filter)(*room));

        if let Some(sort_fn) = sort_fn {
            let mut filtered_invited_rooms = filtered_invited_rooms_iter.collect::<Vec<_>>();
            filtered_invited_rooms.sort_by(|(_, room_a), (_, room_b)| sort_fn(*room_a, *room_b));
            filtered_invited_rooms
                .into_iter()
                .map(|(room_id, _)| room_id.clone())
                .collect()
        } else {
            filtered_invited_rooms_iter
                .map(|(room_id, _)| room_id.clone())
                .collect()
        }
    }

    /// Generates the lists of displayed direct rooms and displayed regular rooms
    /// based on the current filter and the given sort function.
    fn generate_displayed_joined_rooms(
        &self,
        sort_fn: Option<&SortFn>,
    ) -> (Vec<OwnedRoomId>, Vec<OwnedRoomId>) {
        let mut new_displayed_regular_rooms = Vec::new();
        let mut new_displayed_direct_rooms = Vec::new();
        let mut push_room = |room_id: &OwnedRoomId, jr: &JoinedRoomInfo| {
            let room_id = room_id.clone();
            if jr.is_direct {
                new_displayed_direct_rooms.push(room_id);
            } else {
                new_displayed_regular_rooms.push(room_id);
            }
        };

        let filtered_joined_rooms_iter = self
            .all_joined_rooms
            .iter()
            .filter(|(_room_id, room)| (self.display_filter)(*room));

        if let Some(sort_fn) = sort_fn {
            let mut filtered_rooms = filtered_joined_rooms_iter.collect::<Vec<_>>();
            filtered_rooms.sort_by(|(_, room_a), (_, room_b)| sort_fn(*room_a, *room_b));
            for (room_id, jr) in filtered_rooms.into_iter() {
                push_room(room_id, jr)
            }
        } else {
            for (room_id, jr) in filtered_joined_rooms_iter {
                push_room(room_id, jr)
            }
        }

        (new_displayed_regular_rooms, new_displayed_direct_rooms)
    }
}
