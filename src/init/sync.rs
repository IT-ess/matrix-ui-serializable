use std::{iter::Peekable, ops::Deref, sync::Arc};

use anyhow::bail;
use eyeball::Subscriber;
use futures::{StreamExt, future::join_all, pin_mut};
use matrix_sdk::{Client, ruma::OwnedUserId};
use matrix_sdk_ui::{
    RoomListService,
    eyeball_im::{Vector, VectorDiff},
    room_list_service::RoomListItem,
    sync_service::{self, SyncService},
};
use tokio::runtime::Handle;
use tracing::{debug, error, trace};

use crate::{
    init::singletons::{CURRENT_USER_ID, SYNC_SERVICE},
    models::state_updater::StateUpdater,
    room::{
        joined_room::{RoomListServiceRoomInfo, add_new_room, remove_room, update_room},
        rooms_list::{RoomsListUpdate, enqueue_rooms_list_update, handle_rooms_loading_state},
    },
    stores::login_store::FrontendSyncServiceState,
    utils::VecDiff,
};

pub async fn sync(
    client: Client,
    state_updaters: Arc<Box<dyn StateUpdater>>,
) -> anyhow::Result<()> {
    let sync_service = SyncService::builder(client)
        .with_offline_mode()
        .build()
        .await?;

    // Start the sync service
    sync_service.start().await;
    let room_list_service = sync_service.room_list_service();
    let sync_service_state = sync_service.state();
    SYNC_SERVICE
        .set(sync_service)
        .unwrap_or_else(|_| panic!("BUG: SYNC_SERVICE already set!"));

    let all_rooms_list = room_list_service.all_rooms().await?;
    handle_rooms_loading_state(all_rooms_list.loading_state());
    handle_sync_service_state(sync_service_state, state_updaters);

    // TODO: paginate the rooms instead of getting them all
    let (room_diff_stream, room_list_dynamic_entries_controller) =
        all_rooms_list.entries_with_dynamic_adapters(usize::MAX);

    room_list_dynamic_entries_controller.set_filter(Box::new(|_room| true));

    let mut all_known_rooms: Vector<RoomListServiceRoomInfo> = Vector::new();
    let current_user_id = CURRENT_USER_ID.get().cloned();

    pin_mut!(room_diff_stream);
    while let Some(batch) = room_diff_stream.next().await {
        let mut peekable_diffs = batch.into_iter().peekable();
        while let Some(diff) = peekable_diffs.next() {
            let is_reset = matches!(diff, VectorDiff::Reset { .. });
            match diff {
                VectorDiff::Append { values: new_rooms }
                | VectorDiff::Reset { values: new_rooms } => {
                    // Append and Reset are identical, except for Reset first clears all rooms.
                    let _num_new_rooms = new_rooms.len();
                    if is_reset {
                        trace!(
                            "room_list: diff Reset, old length {}, new length {}",
                            all_known_rooms.len(),
                            new_rooms.len()
                        );
                        // Iterate manually so we can know which rooms are being removed.
                        while let Some(room) = all_known_rooms.pop_back() {
                            remove_room(&room);
                        }
                        // ALL_JOINED_ROOMS should already be empty due to successive calls to `remove_room()`,
                        // so this is just a sanity check.
                        crate::room::joined_room::clear_all_rooms();
                        enqueue_rooms_list_update(RoomsListUpdate::ClearRooms);
                        enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::Clear));
                    } else {
                        trace!(
                            "room_list: diff Append, old length {}, adding {} new items",
                            all_known_rooms.len(),
                            _num_new_rooms
                        );
                    }

                    // Parallelize creating each room's RoomListServiceRoomInfo and adding that new room.
                    // We combine `from_room` and `add_new_room` into a single async task per room.
                    let new_room_infos: Vec<RoomListServiceRoomInfo> =
                        join_all(new_rooms.into_iter().map(|room| async {
                            let room_info = RoomListServiceRoomInfo::from_room(
                                room.into_inner(),
                                &current_user_id,
                            )
                            .await;
                            if let Err(e) =
                                add_new_room(&room_info, &room_list_service, false).await
                            {
                                error!(
                                    "Failed to add new room: {}; error: {:?}",
                                    room_info.room_id, e
                                );
                            }
                            room_info
                        }))
                        .await;

                    // Send room order update with the new room IDs
                    let (room_id_refs, room_ids) = {
                        let mut room_id_refs = Vec::with_capacity(new_room_infos.len());
                        let mut room_ids = Vec::with_capacity(new_room_infos.len());
                        for r in &new_room_infos {
                            room_id_refs.push(r.room_id.as_ref());
                            room_ids.push(r.room_id.clone());
                        }
                        (room_id_refs, room_ids)
                    };
                    if !room_ids.is_empty() {
                        enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                            VecDiff::Append { values: room_ids },
                        ));
                        room_list_service.subscribe_to_rooms(&room_id_refs).await;
                        all_known_rooms.extend(new_room_infos);
                    }
                }
                VectorDiff::Clear => {
                    trace!("room_list: diff Clear");
                    all_known_rooms.clear();
                    crate::room::joined_room::clear_all_rooms();
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::Clear));
                    enqueue_rooms_list_update(RoomsListUpdate::ClearRooms);
                }
                VectorDiff::PushFront { value: new_room } => {
                    trace!("room_list: diff PushFront");
                    let new_room = RoomListServiceRoomInfo::from_room(
                        new_room.into_inner(),
                        &CURRENT_USER_ID.get().cloned(),
                    )
                    .await;
                    let room_id = new_room.room_id.clone();
                    add_new_room(&new_room, &room_list_service, true).await?;
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                        VecDiff::PushFront { value: room_id },
                    ));
                    all_known_rooms.push_front(new_room);
                }
                VectorDiff::PushBack { value: new_room } => {
                    trace!("room_list: diff PushBack");
                    let new_room = RoomListServiceRoomInfo::from_room(
                        new_room.into_inner(),
                        &CURRENT_USER_ID.get().cloned(),
                    )
                    .await;
                    let room_id = new_room.room_id.clone();
                    add_new_room(&new_room, &room_list_service, true).await?;
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                        VecDiff::PushBack { value: room_id },
                    ));
                    all_known_rooms.push_back(new_room);
                }
                remove_diff @ VectorDiff::PopFront => {
                    trace!("room_list: diff PopFront");
                    if let Some(room) = all_known_rooms.pop_front() {
                        enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                            VecDiff::PopFront,
                        ));
                        optimize_remove_then_add_into_update(
                            remove_diff,
                            &room,
                            &mut peekable_diffs,
                            &mut all_known_rooms,
                            &room_list_service,
                            &CURRENT_USER_ID.get().cloned(),
                        )
                        .await?;
                    }
                }
                remove_diff @ VectorDiff::PopBack => {
                    trace!("room_list: diff PopBack");
                    if let Some(room) = all_known_rooms.pop_back() {
                        enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                            VecDiff::PopBack,
                        ));
                        optimize_remove_then_add_into_update(
                            remove_diff,
                            &room,
                            &mut peekable_diffs,
                            &mut all_known_rooms,
                            &room_list_service,
                            &CURRENT_USER_ID.get().cloned(),
                        )
                        .await?;
                    }
                }
                VectorDiff::Insert {
                    index,
                    value: new_room,
                } => {
                    trace!("room_list: diff Insert at {index}");
                    let new_room = RoomListServiceRoomInfo::from_room(
                        new_room.into_inner(),
                        &CURRENT_USER_ID.get().cloned(),
                    )
                    .await;
                    let room_id = new_room.room_id.clone();
                    add_new_room(&new_room, &room_list_service, true).await?;
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::Insert {
                        index,
                        value: room_id,
                    }));
                    all_known_rooms.insert(index, new_room);
                }
                VectorDiff::Set {
                    index,
                    value: changed_room,
                } => {
                    trace!("room_list: diff Set at {index}");
                    let changed_room = RoomListServiceRoomInfo::from_room(
                        changed_room.into_inner(),
                        &current_user_id,
                    )
                    .await;
                    if let Some(old_room) = all_known_rooms.get(index) {
                        update_room(old_room, &changed_room, &room_list_service).await?;
                    } else {
                        error!("BUG: room list diff: Set index {index} was out of bounds.");
                    }
                    // Send order update (room ID at this index may have changed)
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::Set {
                        index,
                        value: changed_room.room_id.clone(),
                    }));
                    all_known_rooms.set(index, changed_room);
                }
                remove_diff @ VectorDiff::Remove { index } => {
                    trace!("room_list: diff Remove at {index}");
                    if index < all_known_rooms.len() {
                        let room = all_known_rooms.remove(index);
                        enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                            VecDiff::Remove { index },
                        ));
                        optimize_remove_then_add_into_update(
                            remove_diff,
                            &room,
                            &mut peekable_diffs,
                            &mut all_known_rooms,
                            &room_list_service,
                            &current_user_id,
                        )
                        .await?;
                    } else {
                        error!(
                            "BUG: room_list: diff Remove index {index} out of bounds, len {}",
                            all_known_rooms.len()
                        );
                    }
                }
                VectorDiff::Truncate { length } => {
                    trace!("room_list: diff Truncate to {length}");
                    // Iterate manually so we can know which rooms are being removed.
                    while all_known_rooms.len() > length {
                        if let Some(room) = all_known_rooms.pop_back() {
                            remove_room(&room);
                        }
                    }
                    all_known_rooms.truncate(length); // sanity check
                    enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(
                        VecDiff::Truncate { length },
                    ));
                }
            }
        }
    }

    bail!("room list service sync loop ended unexpectedly")
}

/// Attempts to optimize a common RoomListService operation of remove + add.
///
/// If a `Remove` diff (or `PopBack` or `PopFront`) is immediately followed by
/// an `Insert` diff (or `PushFront` or `PushBack`) for the same room,
/// we can treat it as a simple `Set` operation, in which we call `update_room()`.
/// This is much more efficient than removing the room and then adding it back.
///
/// This tends to happen frequently in order to change the room's state
/// or to "sort" the room list by changing its positional order.
async fn optimize_remove_then_add_into_update(
    remove_diff: VectorDiff<RoomListItem>,
    room: &RoomListServiceRoomInfo,
    peekable_diffs: &mut Peekable<impl Iterator<Item = VectorDiff<RoomListItem>>>,
    all_known_rooms: &mut Vector<RoomListServiceRoomInfo>,
    room_list_service: &RoomListService,
    current_user_id: &Option<OwnedUserId>,
) -> anyhow::Result<()> {
    let next_diff_was_handled: bool;
    match peekable_diffs.peek() {
        Some(VectorDiff::Insert {
            index: insert_index,
            value: new_room,
        }) if room.room_id == new_room.room_id() => {
            trace!(
                "Optimizing {remove_diff:?} + Insert({insert_index}) into Update for room {}",
                room.room_id
            );
            let new_room =
                RoomListServiceRoomInfo::from_room_ref(new_room.deref(), current_user_id).await;
            update_room(room, &new_room, room_list_service).await?;
            // Send order update for the insert
            enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::Insert {
                index: *insert_index,
                value: new_room.room_id.clone(),
            }));
            all_known_rooms.insert(*insert_index, new_room);
            next_diff_was_handled = true;
        }
        Some(VectorDiff::PushFront { value: new_room }) if room.room_id == new_room.room_id() => {
            trace!(
                "Optimizing {remove_diff:?} + PushFront into Update for room {}",
                room.room_id
            );
            let new_room =
                RoomListServiceRoomInfo::from_room_ref(new_room.deref(), current_user_id).await;
            update_room(room, &new_room, room_list_service).await?;
            // Send order update for the push front
            enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::PushFront {
                value: new_room.room_id.clone(),
            }));
            all_known_rooms.push_front(new_room);
            next_diff_was_handled = true;
        }
        Some(VectorDiff::PushBack { value: new_room }) if room.room_id == new_room.room_id() => {
            trace!(
                "Optimizing {remove_diff:?} + PushBack into Update for room {}",
                room.room_id
            );
            let new_room =
                RoomListServiceRoomInfo::from_room_ref(new_room.deref(), current_user_id).await;
            update_room(room, &new_room, room_list_service).await?;
            // Send order update for the push back
            enqueue_rooms_list_update(RoomsListUpdate::RoomOrderUpdate(VecDiff::PushBack {
                value: new_room.room_id.clone(),
            }));
            all_known_rooms.push_back(new_room);
            next_diff_was_handled = true;
        }
        _ => next_diff_was_handled = false,
    }
    if next_diff_was_handled {
        peekable_diffs.next(); // consume the next diff
    } else {
        remove_room(room);
    }
    Ok(())
}

pub fn handle_sync_service_state(
    mut sync_service_state: Subscriber<sync_service::State>,
    state_updaters: Arc<Box<dyn StateUpdater>>,
) {
    Handle::current().spawn(async move {
        while let Some(state) = sync_service_state.next().await {
            debug!("Sync service changed state: {state:?}");

            let _ = state_updaters.update_sync_service(FrontendSyncServiceState::new(state));
        }
    });
}
