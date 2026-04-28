use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::bail;
use futures::{StreamExt, pin_mut};
use matrix_sdk::{
    Client, RoomMemberships,
    ruma::{
        RoomOrAliasId,
        api::client::{
            profile::{AvatarUrl, DisplayName},
            receipt::create_receipt::v3::ReceiptType,
            room::create_room,
        },
        matrix_uri::MatrixId,
    },
};
use matrix_sdk_ui::timeline::{RoomExt, TimelineFocus, TimelineReadReceiptTracking};
use tokio::{
    runtime::Handle,
    sync::{
        Mutex,
        mpsc::{self, UnboundedReceiver},
        watch,
    },
    task::JoinHandle,
};
use tracing::{debug, error, info, trace, warn};

use crate::{
    events::timeline::{
        PaginationDirection, PerTimelineDetails, TimelineKind, TimelineUpdate,
        timeline_subscriber_handler,
    },
    init::singletons::{
        CLIENT, CURRENT_USER_ID, UIUpdateMessage, broadcast_event, get_event_bridge,
    },
    models::{
        async_requests::{MatrixRequest, submit_async_request},
        events::{
            EmitEvent, MatrixUpdateCurrentActiveRoom, ToastNotificationRequest,
            ToastNotificationVariant,
        },
        profile::ProfileModel,
        state_updater::StateUpdater,
    },
    room::{
        joined_room::{UnreadMessageCount, get_timeline, get_timeline_and_sender},
        notifications::{enqueue_toast_notification, process_toast_notifications},
        rooms_list::{
            RoomsCollectionStatus, RoomsList, RoomsListUpdate, enqueue_rooms_list_update,
        },
    },
    user::{
        user_power_level::UserPowerLevels,
        user_profile::{
            UserProfile, UserProfileUpdate, enqueue_user_profile_update,
            process_user_profile_updates,
        },
    },
    utils::debounce_broadcast,
};

/// The main loop that actually uses a Matrix client
pub async fn async_main_loop(
    client: Client,
    state_updaters: Arc<Box<dyn StateUpdater>>,
    room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
) -> anyhow::Result<()> {
    let logged_in_user_id = client
        .user_id()
        .expect("BUG: client.user_id() returned None after successful login!");
    let status = RoomsCollectionStatus::Loading(format!(
        "Logged in as {}.\n → Loading rooms...",
        logged_in_user_id
    ));
    enqueue_toast_notification(ToastNotificationRequest::new(
        format!("Logged in as {logged_in_user_id}."),
        None,
        ToastNotificationVariant::Info,
    ));
    enqueue_rooms_list_update(RoomsListUpdate::Status { status });

    // Listen for updates to the ignored user list.
    // handle_ignore_user_list_subscriber(client.clone());

    Handle::current().spawn(ui_worker(state_updaters.clone(), room_update_receiver));

    crate::init::sync::sync(client, state_updaters).await?;

    bail!("room list service sync loop ended unexpectedly")
}

/// The entry point for an async worker thread that can run async tasks.
///
/// All this thread does is wait for [`MatrixRequests`] from the main UI-driven non-async thread(s)
/// and then executes them within an async runtime context.
pub async fn async_worker(
    mut request_receiver: UnboundedReceiver<MatrixRequest>,
) -> anyhow::Result<()> {
    debug!("Started async_worker task.");
    // The async tasks that are spawned to subscribe to changes in our own user's read receipts for each timeline.
    let mut subscribers_own_user_read_receipts: HashMap<TimelineKind, JoinHandle<()>> =
        HashMap::new();

    while let Some(request) = request_receiver.recv().await {
        match request {
            MatrixRequest::PaginateTimeline {
                timeline_kind,
                num_events,
                direction,
            } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                // Spawn a new async task that will make the actual pagination request.
                let _paginate_task = Handle::current().spawn(async move {
                    debug!("Starting {direction} pagination request for room {timeline_kind}...");
                    sender.send(TimelineUpdate::PaginationRunning(direction)).unwrap();
                    broadcast_event(UIUpdateMessage::RefreshUI);

                    let res = if direction == PaginationDirection::Forwards {
                        timeline.paginate_forwards(num_events).await
                    } else {
                        timeline.paginate_backwards(num_events).await
                    };

                    match res {
                        Ok(fully_paginated) => {
                            debug!("Completed {direction} pagination request for room {timeline_kind}, hit {} of timeline? {}",
                                if direction == PaginationDirection::Forwards { "end" } else { "start" },
                                if fully_paginated { "yes" } else { "no" },
                            );
                            sender.send(TimelineUpdate::PaginationIdle {
                                fully_paginated,
                                direction,
                            }).unwrap();
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        }
                        Err(error) => {
                            warn!("Error sending {direction} pagination request for room {timeline_kind}: {error:?}");
                            sender.send(TimelineUpdate::PaginationError {
                                error,
                                direction,
                            }).unwrap();
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        }
                    }
                });
            }

            MatrixRequest::EditMessage {
                timeline_kind,
                timeline_event_item_id,
                edited_content,
            } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                // Spawn a new async task that will make the actual edit request.
                let _edit_task = Handle::current().spawn(async move {
                    debug!("Sending request to edit message {timeline_event_item_id:?} in {timeline_kind}...");
                    let result = timeline.edit(&timeline_event_item_id, edited_content).await;
                    match result {
                        Ok(_) => debug!(
                            "Successfully edited message {timeline_event_item_id:?} in room {timeline_kind}."
                        ),
                        Err(ref e) => warn!(
                            "Error editing message {timeline_event_item_id:?} in room {timeline_kind}: {e:?}"
                        ),
                    }
                    sender
                        .send(TimelineUpdate::MessageEdited {
                            timeline_event_item_id,
                            result,
                        })
                        .unwrap();
                    broadcast_event(UIUpdateMessage::RefreshUI);
                });
            }

            MatrixRequest::CreateThreadTimeline {
                room_id,
                thread_root_event_id,
            } => {
                let main_room_timeline = {
                    let Some(arc) = crate::room::joined_room::try_get_room_details(&room_id) else {
                        error!("BUG: room info not found for get room members request {room_id}");
                        continue;
                    };
                    let mut room_info = arc.lock().unwrap();

                    if room_info
                        .thread_timelines
                        .contains_key(&thread_root_event_id)
                    {
                        continue;
                    }
                    let newly_pending = room_info
                        .pending_thread_timelines
                        .insert(thread_root_event_id.clone());
                    if !newly_pending {
                        continue;
                    }
                    room_info.main_timeline.timeline.clone()
                };

                let _create_thread_timeline_task = Handle::current().spawn(async move {
                    debug!("Creating thread-focused timeline for room {room_id}, thread {thread_root_event_id}...");
                    let build_result = main_room_timeline.room()
                        .timeline_builder()
                        .with_focus(TimelineFocus::Thread {
                            root_event_id: thread_root_event_id.clone(),
                        })
                        .track_read_marker_and_receipts(TimelineReadReceiptTracking::AllEvents)
                        .build()
                        .await;

                    match build_result {
                        Ok(thread_timeline) => {
                            let Some(arc) = crate::room::joined_room::try_get_room_details(&room_id)
                            else {
                                return;
                            };
                            let mut room_info = arc.lock().unwrap();
                            debug!("Successfully created thread-focused timeline for room {room_id}, thread {thread_root_event_id}.");
                            let thread_timeline = Arc::new(thread_timeline);
                            let (timeline_update_sender, timeline_update_receiver) = crossbeam_channel::unbounded();
                            let (request_sender, request_receiver) = watch::channel(Vec::new());
                            let timeline_subscriber_handler_task = Handle::current().spawn(
                                timeline_subscriber_handler(
                                    main_room_timeline.room().clone(),
                                    thread_timeline.clone(),
                                    timeline_update_sender.clone(),
                                    request_receiver,
                                    Some(thread_root_event_id.clone()),
                                )
                            );
                            room_info
                                .pending_thread_timelines
                                .remove(&thread_root_event_id);
                            room_info.thread_timelines.insert(
                                thread_root_event_id.clone(),
                                PerTimelineDetails {
                                    timeline: thread_timeline,
                                    timeline_update_sender,
                                    timeline_singleton_endpoints: Some((
                                        timeline_update_receiver,
                                        request_sender,
                                    )),
                                    timeline_subscriber_handler_task,
                                },
                            );
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        }
                        Err(error) => {
                            error!("Failed to create thread-focused timeline for room {room_id}, thread {thread_root_event_id}: {error}");
                            if let Some(arc) = crate::room::joined_room::try_get_room_details(&room_id)
                            {
                            let mut room_info = arc.lock().unwrap();
                            room_info
                                .pending_thread_timelines
                                .remove(&thread_root_event_id);
                            }
                            enqueue_toast_notification(
                                ToastNotificationRequest::new(
                                format!("Failed to create thread-focused timeline. Please retry opening the thread again later.\n\nError: {error}"),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        }
                    }
                });
            }

            MatrixRequest::FetchDetailsForEvent {
                timeline_kind,
                event_id,
            } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                // Spawn a new async task that will make the actual fetch request.
                let _fetch_task = Handle::current().spawn(async move {
                    debug!("Sending request to fetch details for event {event_id} in room {timeline_kind}...");
                    let result = timeline.fetch_details_for_event(&event_id).await;
                    match result {
                        Ok(_) => {
                            debug!("Successfully fetched details for event {event_id} in room {timeline_kind}.");
                        }
                        Err(ref _e) => {
                            // warn!("Error fetching details for event {event_id} in room {room_id}: {e:?}");
                        }
                    }
                    sender
                        .send(TimelineUpdate::EventDetailsFetched { event_id, result })
                        .unwrap();
                    broadcast_event(UIUpdateMessage::RefreshUI);
                });
            }

            MatrixRequest::SyncRoomMemberList { timeline_kind } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                // Spawn a new async task that will make the actual fetch request.
                let _fetch_task = Handle::current().spawn(async move {
                    debug!("Sending sync room members request for room {timeline_kind}...");
                    timeline.fetch_members().await;
                    debug!("Completed sync room members request for room {timeline_kind}.");
                    sender.send(TimelineUpdate::RoomMembersSynced).unwrap();

                    // Get room members details after the list sync.
                    submit_async_request(MatrixRequest::GetRoomMembers {
                        timeline_kind,
                        memberships: RoomMemberships::JOIN,
                        local_only: false,
                    });
                    broadcast_event(UIUpdateMessage::RefreshUI);
                });
            }
            MatrixRequest::JoinRoom { room_id } => {
                let Some(client) = CLIENT.get() else { continue };
                let _join_room_task = Handle::current().spawn(async move {
                    debug!("Sending request to join room {room_id}...");
                    if let Some(room) = client.get_room(&room_id) {
                        match room.join().await {
                            Ok(()) => {
                                debug!("Successfully joined room {room_id}.");
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Successfully joined room {room_id}."),
                                    None,
                                    ToastNotificationVariant::Success,
                                ));
                            }
                            Err(e) => {
                                error!("Error joining room {room_id}: {e:?}");
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Error joining room {room_id}: {e:?}"),
                                    None,
                                    ToastNotificationVariant::Error,
                                ));
                            }
                        }
                    } else {
                        error!("BUG: client could not get room with ID {room_id}");
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            format!("BUG: client could not get room with ID {room_id}"),
                            None,
                            ToastNotificationVariant::Error,
                        ));
                    }
                });
            }
            MatrixRequest::LeaveRoom { room_id } => {
                let Some(client) = CLIENT.get() else { continue };
                let _leave_room_task = Handle::current().spawn(async move {
                    debug!("Sending request to leave room {room_id}...");
                    if let Some(room) = client.get_room(&room_id) {
                        match room.leave().await {
                            Ok(()) => {
                                debug!("Successfully left room {room_id}.");
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Successfully left room {room_id}."),
                                    None,
                                    ToastNotificationVariant::Success,
                                ));
                            }
                            Err(e) => {
                                error!("Error leaving room {room_id}: {e:?}");
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Error leaving room {room_id}: {e:?}"),
                                    None,
                                    ToastNotificationVariant::Error,
                                ));
                            }
                        }
                    } else {
                        error!("BUG: client could not get room with ID {room_id}");
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            "Client couldn't locate room to leave it.".to_owned(),
                            None,
                            ToastNotificationVariant::Error,
                        ));
                    }
                });
            }
            MatrixRequest::GetRoomMembers {
                timeline_kind,
                memberships,
                local_only,
            } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                let _get_members_task = Handle::current().spawn(async move {
                    let room = timeline.room();

                    if local_only {
                        if let Ok(members) = room.members_no_sync(memberships).await {
                            debug!(
                                "Got {} members from cache for room {}",
                                members.len(),
                                timeline_kind
                            );
                            sender
                                .send(TimelineUpdate::RoomMembersListFetched { members })
                                .unwrap();
                        }
                    } else if let Ok(members) = room.members(memberships).await {
                        debug!(
                            "Successfully fetched {} members from server for room {}",
                            members.len(),
                            timeline_kind
                        );
                        sender
                            .send(TimelineUpdate::RoomMembersListFetched { members })
                            .unwrap();
                    }

                    broadcast_event(UIUpdateMessage::RefreshUI);
                });
            }

            MatrixRequest::GetUserProfile {
                user_id,
                room_id,
                local_only,
                sender,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let _fetch_task = Handle::current().spawn(async move {

                    let mut update = None;

                    if let Some(room_id) = room_id.as_ref() {
                        if let Some(room) = client.get_room(room_id) {
                            let member = if local_only {
                                room.get_member_no_sync(&user_id).await
                            } else {
                                room.get_member(&user_id).await
                            };
                            if let Ok(Some(room_member)) = member {
                                update = Some(UserProfileUpdate::Full {
                                    new_profile: UserProfile {
                                        username: room_member.display_name().map(|u| u.to_owned()),
                                        user_id: user_id.clone(),
                                        avatar: room_member.avatar_url().map(|u| u.to_owned()),
                                    },
                                    room_id: room_id.to_owned(),
                                    room_member,
                                });
                            } else {
                                warn!("User profile request: user {user_id} was not a member of room {room_id}");
                            }
                        } else {
                            warn!("User profile request: client could not get room with ID {room_id}");
                        }
                    }

                    if !local_only {
                        if update.is_none() {
                            if let Ok(response) = client.account().fetch_user_profile_of(&user_id).await {
                                update = Some(UserProfileUpdate::UserProfileOnly(
                                    UserProfile {
                                        username: response.get_static::<DisplayName>().ok().flatten(),
                                        user_id: user_id.clone(),
                                        avatar: response.get_static::<AvatarUrl>()
                                            .ok()
                                            .unwrap_or_default(),
                                    }
                                ));
                            } else {
                                warn!("User profile request: client could not get user with ID {user_id}");
                            }
                        }

                        match update.as_mut() {
                            Some(UserProfileUpdate::Full { new_profile: UserProfile { username, .. }, .. }) if username.is_none() => {
                                if let Ok(response) = client.account().fetch_user_profile_of(&user_id).await {
                                    *username = response.get_static::<DisplayName>().ok().flatten();
                                }
                            }
                            _ => { }
                        }
                    }

                    if let Some(upd) = update {
                        if let Some(sender) = sender {
                         let _ = sender.send(upd.get_user_profile_from_update().cloned());
                        }
                        debug!("Successfully completed get user profile request: user: {user_id}, room: {room_id:?}, local_only: {local_only}.");
                        enqueue_user_profile_update(upd);
                    } else {
                        error!("Failed to get user profile: user: {user_id}, room: {room_id:?}, local_only: {local_only}.");
                    }
                });
            }
            MatrixRequest::GetNumberUnreadMessages { timeline_kind } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };
                let _get_unreads_task = Handle::current().spawn(async move {
                    match sender.send(TimelineUpdate::NewUnreadMessagesCount(
                        UnreadMessageCount::Known(timeline.room().num_unread_messages())
                    )) {
                        Ok(_) => {
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        },
                        Err(e) => error!("Failed to send timeline update: {e:?} for GetNumberUnreadMessages request for {timeline_kind}")
                    }
                    if let TimelineKind::MainRoom { room_id } = timeline_kind {
                        enqueue_rooms_list_update(RoomsListUpdate::UpdateNumUnreadMessages {
                            room_id,
                            is_marked_unread: timeline.room().is_marked_unread(),
                            unread_messages: UnreadMessageCount::Known(timeline.room().num_unread_messages()),
                            unread_mentions: timeline.room().num_unread_mentions(),
                        });
                    }
                });
            }
            MatrixRequest::IgnoreUser {
                ignore,
                room_member,
                room_id,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let _ignore_task = Handle::current().spawn(async move {
                    let user_id = room_member.user_id();
                    debug!("Sending request to {}ignore user: {user_id}...", if ignore { "" } else { "un" });
                    let ignore_result = if ignore {
                        room_member.ignore().await
                    } else {
                        room_member.unignore().await
                    };

                    debug!("{} user {user_id} {}",
                        if ignore { "Ignoring" } else { "Unignoring" },
                        if ignore_result.is_ok() { "succeeded." } else { "failed." },
                    );

                    if ignore_result.is_err() {
                        return;
                    }

                    // We need to re-acquire the `RoomMember` object now that its state
                    // has changed, i.e., the user has been (un)ignored.
                    // We then need to send an update to replace the cached `RoomMember`
                    // with the now-stale ignored state.
                    if let Some(room) = client.get_room(&room_id) && let Ok(Some(new_room_member)) = room.get_member(user_id).await {
                            debug!("Enqueueing user profile update for user {user_id}, who went from {}ignored to {}ignored.",
                                if room_member.is_ignored() { "" } else { "un" },
                                if new_room_member.is_ignored() { "" } else { "un" },
                            );
                            enqueue_user_profile_update(UserProfileUpdate::RoomMemberOnly {
                                room_id: room_id.clone(),
                                room_member: new_room_member,
                            });
                    }

                    // After successfully (un)ignoring a user, all timelines are fully cleared by the Matrix SDK.
                    // Therefore, we need to re-fetch all timelines for all rooms,
                    // and currently the only way to actually accomplish this is via pagination.
                    // See: <https://github.com/matrix-org/matrix-rust-sdk/issues/1703#issuecomment-2250297923>
                    //
                    // Note that here we only proactively re-paginate the *current* room
                    // (the one being viewed by the user when this ignore request was issued),
                    // and all other rooms will be re-paginated in `handle_ignore_user_list_subscriber()`.`
                    submit_async_request(MatrixRequest::PaginateTimeline {
                        timeline_kind: TimelineKind::MainRoom { room_id },
                        num_events: 50,
                        direction: PaginationDirection::Backwards,
                    });
                });
            }

            MatrixRequest::SendTypingNotice { room_id, typing } => {
                let Some(room) = CLIENT.get().and_then(|c| c.get_room(&room_id)) else {
                    error!("BUG: client/room not found for typing notice request {room_id}");
                    continue;
                };
                let _typing_task = Handle::current().spawn(async move {
                    if let Err(e) = room.typing_notice(typing).await {
                        warn!("Failed to send typing notice to room {room_id}: {e:?}");
                    }
                });
            }

            MatrixRequest::SubscribeToTypingNotices { room_id, subscribe } => {
                let (room, timeline_update_sender, mut typing_notice_receiver) = {
                    let Some(room_info) = crate::room::joined_room::try_get_room_details(&room_id)
                    else {
                        error!(
                            "BUG: room info not found for subscribe to typing notices request, room {room_id}"
                        );
                        continue;
                    };
                    let (room, recv) = if subscribe {
                        if room_info.lock().unwrap().typing_notice_subscriber.is_some() {
                            debug!("Note: room {room_id} is already subscribed to typing notices.");
                            continue;
                        } else {
                            let Some(room) = CLIENT.get().and_then(|c| c.get_room(&room_id)) else {
                                warn!(
                                    "BUG: client/room not found when subscribing to typing notices request, room: {room_id}"
                                );
                                continue;
                            };
                            let (drop_guard, recv) = room.subscribe_to_typing_notifications();

                            let mut lock = room_info.lock().unwrap();
                            lock.typing_notice_subscriber = Some(drop_guard);
                            (room, recv)
                        }
                    } else {
                        let mut lock = room_info.lock().unwrap();
                        lock.typing_notice_subscriber.take();
                        continue;
                    };
                    // Here: we don't have an existing subscriber running, so we fall through and start one.
                    (
                        room,
                        room_info
                            .lock()
                            .unwrap()
                            .main_timeline
                            .timeline_update_sender
                            .clone(),
                        recv,
                    )
                };

                let _typing_notices_task = Handle::current().spawn(async move {
                    while let Ok(user_ids) = typing_notice_receiver.recv().await {
                        debug!("Received typing notifications for room {room_id}: {user_ids:?}");
                        let mut users = Vec::with_capacity(user_ids.len());
                        for user_id in user_ids {
                            users.push(
                                room.get_member_no_sync(&user_id)
                                    .await
                                    .ok()
                                    .flatten()
                                    .and_then(|m| m.display_name().map(|d| d.to_owned()))
                                    .unwrap_or_else(|| user_id.to_string())
                            );
                        }
                        if let Err(e) = timeline_update_sender.send(TimelineUpdate::TypingUsers { users }) {
                            warn!("Error: timeline update sender couldn't send the list of typing users: {e:?}");
                        }
                        broadcast_event(UIUpdateMessage::RefreshUI);
                    }
                    debug!("Note: typing notifications recv loop has ended for room {}", room_id);
                });
            }
            MatrixRequest::SubscribeToOwnUserReadReceiptsChanged {
                timeline_kind,
                subscribe,
            } => {
                if !subscribe {
                    if let Some(task_handler) =
                        subscribers_own_user_read_receipts.remove(&timeline_kind)
                    {
                        task_handler.abort();
                    }
                    continue;
                }
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                let timeline_kind_clone = timeline_kind.clone();
                let subscribe_own_read_receipt_task = Handle::current().spawn(async move {
                    let update_receiver = timeline.subscribe_own_user_read_receipts_changed().await;
                    pin_mut!(update_receiver);
                    if let Some(client_user_id) = CURRENT_USER_ID.get() {
                        if let Some((event_id, receipt)) = timeline.latest_user_read_receipt(client_user_id).await {
                            trace!("Received own user read receipt for {timeline_kind}: {receipt:?}, event ID: {event_id:?}");
                            if sender.send(TimelineUpdate::OwnUserReadReceipt(receipt)).is_err() {
                                error!("Failed to send own user read receipt to UI.");
                            }
                        }

                        while update_receiver.next().await.is_some() {
                            if let Some((_, receipt)) = timeline.latest_user_read_receipt(client_user_id).await {
                                if sender.send(TimelineUpdate::OwnUserReadReceipt(receipt)).is_err() {
                                    error!("Failed to send own user read receipt to UI.");
                                }
                                // When read receipts change (from other devices), update unread count
                                let unread_count = timeline.room().num_unread_messages();
                                let unread_mentions = timeline.room().num_unread_mentions();
                                if sender.send(TimelineUpdate::NewUnreadMessagesCount(
                                    UnreadMessageCount::Known(unread_count)
                                )).is_err() {
                                    error!("Failed to send unread message count update to UI.");
                                }
                                if let TimelineKind::MainRoom { room_id } = &timeline_kind {
                                    // Update the rooms list with new unread counts
                                    enqueue_rooms_list_update(RoomsListUpdate::UpdateNumUnreadMessages {
                                        room_id: room_id.clone(),
                                        is_marked_unread: timeline.room().is_marked_unread(),
                                        unread_messages: UnreadMessageCount::Known(unread_count),
                                        unread_mentions,
                                    });
                                }
                            }
                        }
                    }
                });
                subscribers_own_user_read_receipts
                    .insert(timeline_kind_clone, subscribe_own_read_receipt_task);
            }
            MatrixRequest::ResolveRoomAlias(room_alias) => {
                let Some(client) = CLIENT.get() else { continue };
                let _resolve_task = Handle::current().spawn(async move {
                    debug!("Sending resolve room alias request for {room_alias}...");
                    let res = client.resolve_room_alias(&room_alias).await;
                    debug!("Resolved room alias {room_alias} to: {res:?}");
                    //TODO: Send the resolved room alias back to the UI thread somehow.
                });
            }
            MatrixRequest::FetchMedia {
                media_request,
                content_sender,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let media = client.media();

                let _fetch_task = Handle::current().spawn(async move {
                    debug!("Sending fetch media request for {media_request:?}...");
                    let res = media.get_media_content(&media_request, true).await;
                    if let Err(e) = content_sender.send(res) {
                        error!("Cannot send media content. {e:?}");
                    };
                });
            }
            MatrixRequest::SendMessage {
                timeline_kind,
                message,
                replied_to_id,
            } => {
                // TODO: use this timeline `_sender` once we support sending-message status/operations in the UI.
                let Some((timeline, _sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("BUG: {timeline_kind} not found for send message request");
                    continue;
                };

                // Spawn a new async task that will send the actual message.
                let _send_message_task = Handle::current().spawn(async move {
                    debug!("Sending message to room {timeline_kind}: {message:?}...");
                    if let Some(replied_event_id) = replied_to_id {
                        match timeline.send_reply(message.into(), replied_event_id).await {
                            Ok(_send_handle) => {
                                debug!("Sent reply message to room {timeline_kind}.")
                            }
                            Err(_e) => {
                                warn!(
                                    "Failed to send reply message to room {timeline_kind}: {_e:?}"
                                );
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Failed to send reply: {_e}"),
                                    None,
                                    ToastNotificationVariant::Error,
                                ));
                            }
                        }
                    } else {
                        match timeline.send(message.into()).await {
                            Ok(_send_handle) => debug!("Sent message to room {timeline_kind}."),
                            Err(e) => {
                                warn!("Failed to send message to room {timeline_kind}: {e:?}");
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Failed to send message. Error: {e}"),
                                    None,
                                    ToastNotificationVariant::Error,
                                ));
                            }
                        }
                    }
                    broadcast_event(UIUpdateMessage::RefreshUI);
                });
            }

            MatrixRequest::ReadReceipt {
                timeline_kind,
                event_id,
                receipt_type,
            } => {
                let Some(timeline) = get_timeline(&timeline_kind) else {
                    error!("BUG: {timeline_kind} not found when sending read receipt, {event_id}");
                    continue;
                };

                let _send_rr_task = Handle::current().spawn(async move {
                    match timeline.send_single_receipt(receipt_type, event_id.clone()).await {
                        Ok(sent) => debug!("{} read receipt to room {timeline_kind} for event {event_id}", if sent { "Sent" } else { "Already sent" }),
                        Err(_e) => warn!("Failed to send read receipt to room {timeline_kind} for event {event_id}; error: {_e:?}"),
                    }
                    if let TimelineKind::MainRoom { room_id } = timeline_kind {
                        // Also update the number of unread messages in the room.
                        enqueue_rooms_list_update(RoomsListUpdate::UpdateNumUnreadMessages {
                            room_id,
                            is_marked_unread: timeline.room().is_marked_unread(),
                            unread_messages: UnreadMessageCount::Known(timeline.room().num_unread_messages()),
                            unread_mentions: timeline.room().num_unread_mentions()
                        });
                    }
                });
            }

            MatrixRequest::MarkRoomAsRead { timeline_kind } => {
                let Some(timeline) = get_timeline(&timeline_kind) else {
                    error!("BUG: {timeline_kind} not found when marking room as read");
                    continue;
                };
                let _send_frr_task = Handle::current().spawn(async move {
                    match timeline.mark_as_read(ReceiptType::FullyRead).await {
                        Ok(sent) => debug!(
                            "{} fully read receipt to room {timeline_kind}",
                            if sent { "Sent" } else { "Already sent" }
                        ),
                        Err(_e) => warn!(
                            "Failed to send fully read receipt to room {timeline_kind}; error: {_e:?}"
                        ),
                    }
                    if let TimelineKind::MainRoom { room_id } = timeline_kind {
                        // Also update the number of unread messages in the room.
                        enqueue_rooms_list_update(RoomsListUpdate::UpdateNumUnreadMessages {
                            room_id,
                            is_marked_unread: timeline.room().is_marked_unread(),
                            unread_messages: UnreadMessageCount::Known(timeline.room().num_unread_messages()),
                            unread_mentions: timeline.room().num_unread_mentions()
                        });
                    }
                });
            }

            MatrixRequest::GetRoomPowerLevels { timeline_kind } => {
                let Some((timeline, sender)) = get_timeline_and_sender(&timeline_kind) else {
                    trace!("Skipping pagination request for unknown {timeline_kind}");
                    continue;
                };

                let Some(user_id) = CURRENT_USER_ID.get() else {
                    continue;
                };

                let _power_levels_task = Handle::current().spawn(async move {
                    match timeline.room().power_levels().await {
                        Ok(power_levels) => {
                            debug!("Successfully fetched power levels for room {timeline_kind}.");
                            if let Err(e) = sender.send(TimelineUpdate::UserPowerLevels(
                                UserPowerLevels::from(&power_levels, user_id),
                            )) {
                                warn!("Failed to send the result of if user can send message: {e}")
                            }
                            broadcast_event(UIUpdateMessage::RefreshUI);
                        }
                        Err(e) => {
                            warn!("Failed to fetch power levels for room {timeline_kind}: {e:?}");
                        }
                    }
                });
            }
            MatrixRequest::ToggleReaction {
                timeline_kind,
                timeline_event_id,
                reaction,
            } => {
                let Some(timeline) = get_timeline(&timeline_kind) else {
                    error!(
                        "BUG: {timeline_kind} not found when sending reaction, {timeline_event_id:?}"
                    );
                    continue;
                };

                let _toggle_reaction_task = Handle::current().spawn(async move {
                    debug!("Toggle Reaction to room {timeline_kind}: ...");
                    match timeline.toggle_reaction(&timeline_event_id, &reaction).await {
                        Ok(_send_handle) => {
                            broadcast_event(UIUpdateMessage::RefreshUI);
                            debug!("Sent toggle reaction to room {timeline_kind} {reaction}.")
                        },
                        Err(_e) => error!("Failed to send toggle reaction to room {timeline_kind} {reaction}; error: {_e:?}"),
                    }
                });
            }
            MatrixRequest::RedactMessage {
                timeline_kind,
                timeline_event_id,
                reason,
            } => {
                let Some(timeline) = get_timeline(&timeline_kind) else {
                    error!(
                        "BUG: {timeline_kind} not found when redacting message, {timeline_event_id:?}"
                    );
                    continue;
                };

                let _redact_task = Handle::current().spawn(async move {
                    match timeline.redact(&timeline_event_id, reason.as_deref()).await {
                        Ok(()) => {
                            debug!("Successfully redacted message in room {timeline_kind}.");
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                "Event has been redacted successfully.".to_owned(),
                                None,
                                ToastNotificationVariant::Success,
                            ));
                        }
                        Err(e) => {
                            error!("Failed to redact message in {timeline_kind}; error: {e:?}");
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!("Failed to redact message. Error: {e}"),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        }
                    }
                });
            }
            MatrixRequest::GetMatrixRoomLinkPillInfo { matrix_id, via } => {
                let Some(client) = CLIENT.get() else { continue };
                let _fetch_matrix_link_pill_info_task = Handle::current().spawn(async move {
                    let room_or_alias_id: Option<&RoomOrAliasId> = match &matrix_id {
                        MatrixId::Room(room_id) => Some((&**room_id).into()),
                        MatrixId::RoomAlias(room_alias_id) => Some((&**room_alias_id).into()),
                        MatrixId::Event(room_or_alias_id, _event_id) => Some(room_or_alias_id),
                        _ => {
                            warn!("MatrixLinkRoomPillInfoRequest: Unsupported MatrixId type: {matrix_id:?}");
                            return;
                        }
                    };
                    if let Some(room_or_alias_id) = room_or_alias_id {
                        match client.get_room_preview(room_or_alias_id, via).await {
                            Ok(_preview) => {},
                            Err(e) => {
                                error!("Failed to get room link pill info for {room_or_alias_id:?}: {e:?}");
                            }
                        }
                    }
                });
            }
            MatrixRequest::SearchUsers {
                search_term,
                limit,
                content_sender,
            } => {
                let Some(client) = CLIENT.get() else { continue };

                let _fetch_task = Handle::current().spawn(async move {
                    match client.search_users(&search_term, limit).await {
                        Ok(res) => {
                            let users: Vec<ProfileModel> =
                                res.results.iter().cloned().map(|i| i.into()).collect();
                            content_sender.send(Ok(users))
                        }
                        Err(e) => content_sender.send(Err(e.into())),
                    }
                });
            }
            MatrixRequest::CreateDMRoom { user_id } => {
                let Some(client) = CLIENT.get() else { continue };
                let _create_dm_room_task = Handle::current().spawn(async move {
                    match client.create_dm(&user_id).await {
                        Ok(room) => {
                            let event_bridge =
                                get_event_bridge().expect("event bridge should be defined");
                            event_bridge
                                .emit(EmitEvent::NewlyCreatedRoomId(room.room_id().to_owned()));
                            info!("Sucessfully created DM room with user {user_id}");
                        }
                        Err(e) => {
                            error!(
                                "Failed to create DM room for user {user_id}, error: {:?}",
                                e
                            );
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!(
                                    "Failed to create DM room for user {user_id}, error: {e:?}"
                                ),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        }
                    }
                });
            }
            MatrixRequest::CreateRoom {
                room_name,
                room_avatar,
                invited_user_ids,
                topic,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let _create_room_task = Handle::current().spawn(async move {
                    let mut request = create_room::v3::Request::new();
                    request.is_direct = false;
                    request.name = Some(room_name);
                    // We only support private rooms for now.
                    request.visibility = matrix_sdk::ruma::api::client::room::Visibility::Private;
                    request.invite = invited_user_ids;
                    request.preset = Some(create_room::v3::RoomPreset::TrustedPrivateChat);
                    request.room_version = Some(matrix_sdk::ruma::RoomVersionId::V12);
                    request.topic = topic;

                    match client.create_room(request).await {
                        Ok(room) => {
                            info!("Sucessfully created room");
                            if let Err(e) = room.enable_encryption().await {
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    format!("Failed to enable encryption in Room. Error: {e}"),
                                    None,
                                    ToastNotificationVariant::Error,
                                ))
                            } else {
                                info!("Enabled encryption for room with id {}", room.room_id());
                                if let Some(avatar_uri) = room_avatar
                                    && let Err(e) = room.set_avatar_url(&avatar_uri, None).await
                                {
                                    error!(
                                        "Failed to set avatar for room with id {}. Error: {e:?}",
                                        room.room_id()
                                    );
                                    enqueue_toast_notification(ToastNotificationRequest::new(
                                        format!("Failed to set avatar for room, error: {e:?}"),
                                        None,
                                        ToastNotificationVariant::Error,
                                    ));
                                }
                                enqueue_toast_notification(ToastNotificationRequest::new(
                                    "Sucessfully created room".to_owned(),
                                    None,
                                    ToastNotificationVariant::Success,
                                ));
                                let event_bridge =
                                    get_event_bridge().expect("event bridge should be defined");
                                event_bridge
                                    .emit(EmitEvent::NewlyCreatedRoomId(room.room_id().to_owned()));
                            }
                        }
                        Err(e) => {
                            error!("Failed to create room, error: {:?}", e);
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!("Failed to create room, error: {e:?}"),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        }
                    }
                });
            }
            MatrixRequest::InviteUsersInRoom {
                room_id,
                invited_user_ids,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let _invite_task = Handle::current().spawn(async move {
                    let room = client
                        .get_room(&room_id)
                        .expect("Room should be defined if we can invite users in it");
                    for user_id in invited_user_ids {
                        if let Err(e) = room.invite_user_by_id(&user_id).await {
                            error!(
                                "Failed to invite user {user_id} to room {room_id}, error: {:?}",
                                e
                            );
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!("Failed to invite user {user_id} to room {room_id}"),
                                Some(format!("Error: {e:?}")),
                                ToastNotificationVariant::Error,
                            ));
                        }
                    }
                });
            }
            MatrixRequest::KickOrBanUserFromRoom {
                room_id,
                user_id,
                is_ban,
                reason,
            } => {
                let Some(client) = CLIENT.get() else { continue };
                let _kick_task = Handle::current().spawn(async move {
                    let room = client
                        .get_room(&room_id)
                        .expect("Room should be defined if we can ban users from it");
                    if let Err(e) = if is_ban {
                        room.ban_user(&user_id, reason.as_deref()).await
                    } else {
                        room.kick_user(&user_id, reason.as_deref()).await
                    } {
                        error!("Cannot kick or ban user. {e}");
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            format!("Failed to kick or ban user. {e}"),
                            Some(format!("Error: {e:?}")),
                            ToastNotificationVariant::Error,
                        ));
                    } else {
                        submit_async_request(MatrixRequest::SyncRoomMemberList {
                            timeline_kind: TimelineKind::MainRoom { room_id },
                        });
                    }
                });
            }
        }
    }

    error!("async_worker task ended unexpectedly");
    bail!("async_worker task ended unexpectedly")
}

/// Worker that loops to update rooms_list updates in queue
/// currently it handles active_room updates outside the other actions,
/// but maybe I should handle this as every other action
pub async fn ui_worker(
    state_updaters: Arc<Box<dyn StateUpdater>>,
    mut room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
) -> anyhow::Result<()> {
    let rooms_list = Arc::new(Mutex::new(RoomsList::new(state_updaters.clone())));

    // create UI subscriber
    let mut ui_subscriber = debounce_broadcast(
        crate::init::singletons::subscribe_to_events().expect("Couldn't get UI subscriber event"),
        Duration::from_millis(200),
    );

    loop {
        tokio::select! {
            // Handle incoming events from listener
            Some(MatrixUpdateCurrentActiveRoom { thread_root_event_id, room_id, room_name }) = room_update_receiver.recv() => {
                let timeline_kind = if let Some(root) = thread_root_event_id {
                    TimelineKind::Thread { thread_root_event_id: root, room_id }
                } else {
                    TimelineKind::MainRoom { room_id }
                };
                let mut lock = rooms_list.lock().await;
                if let Err(e) = lock.handle_current_active_room(timeline_kind, room_name) {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("Failed to set the current viewed room. Error: {e}"),
                        None,
                        ToastNotificationVariant::Error,
                    ))
                }
            }

            // Listen to UI refresh events
            _ = ui_subscriber.recv() => {
                let mut lock = rooms_list.lock().await;
                lock.handle_rooms_list_updates().await;

                process_user_profile_updates(); // Each time the UI is refreshed we check the profiles update queue.

                let _ = process_toast_notifications().await;
            }
        }
    }
}
