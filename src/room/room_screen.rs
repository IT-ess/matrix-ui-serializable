use std::{collections::BTreeMap, sync::Arc};

use matrix_sdk::ruma::{OwnedEventId, OwnedRoomId, OwnedUserId};
use matrix_sdk_ui::{eyeball_im::Vector, timeline::TimelineItem};
use rangemap::RangeSet;
use serde::Serialize;

use crate::{
    events::timeline::{
        PaginationDirection, TIMELINE_STATES, TimelineUiState, TimelineUpdate,
        take_timeline_endpoints,
    },
    models::{
        async_requests::{MatrixRequest, submit_async_request},
        state_updater::StateUpdater,
    },
    user::user_power_level::UserPowerLevels,
    utils::room_name_or_id,
};

/// The main widget that displays a single Matrix room.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomScreen {
    /// The room ID of the currently-shown room.
    room_id: OwnedRoomId,
    /// The display name of the currently-shown room.
    room_name: String,
    /// The persistent UI-relevant states for the room that this widget is currently displaying.
    tl_state: Option<TimelineUiState>,
    /// Known members of this room
    members: BTreeMap<OwnedUserId, FrontendRoomMember>,
    /// The state updater passed by the adapter
    #[serde(skip)]
    state_updaters: Arc<Box<dyn StateUpdater>>,
}
impl Drop for RoomScreen {
    fn drop(&mut self) {
        // This ensures that the `TimelineUiState` instance owned by this room is *always* returned
        // back to to `TIMELINE_STATES`, which ensures that its UI state(s) are not lost
        // and that other RoomScreen instances can show this room in the future.
        // RoomScreen will be dropped whenever its widget instance is destroyed, e.g.,
        // when a Tab is closed or the app is resized to a different AdaptiveView layout.
        self.hide_timeline();
    }
}

impl RoomScreen {
    pub fn new(
        updaters: Arc<Box<dyn StateUpdater>>,
        room_id: OwnedRoomId,
        room_name: String,
    ) -> Self {
        Self {
            room_id,
            room_name,
            tl_state: None,
            members: BTreeMap::new(),
            state_updaters: updaters,
        }
    }

    fn update_frontend_state(&self) {
        self.state_updaters
            .update_room(self, self.room_id.as_str())
            .expect(&format!(
                "Couldn't update frontend store for room {:?}",
                self.room_name,
            ))
    }

    /// Processes all pending background updates to the currently-shown timeline.
    pub fn process_timeline_updates(&mut self) {
        let curr_first_id: usize = 0; // TODO: replace this dummy value

        let Some(tl) = self.tl_state.as_mut() else {
            return;
        };

        let mut done_loading = false;
        let mut should_continue_backwards_pagination = false;
        let mut num_updates = 0;
        let mut typing_users = Vec::new();
        while let Ok(update) = tl.update_receiver.try_recv() {
            num_updates += 1;
            match update {
                TimelineUpdate::FirstUpdate { initial_items } => {
                    tl.content_drawn_since_last_update.clear();
                    tl.profile_drawn_since_last_update.clear();
                    tl.fully_paginated = false;

                    tl.items = initial_items;
                    done_loading = true;
                }
                TimelineUpdate::NewItems {
                    new_items,
                    changed_indices,
                    clear_cache,
                } => {
                    if new_items.is_empty() {
                        if !tl.items.is_empty() {
                            println!(
                                "Timeline::handle_event(): timeline (had {} items) was cleared for room {}",
                                tl.items.len(),
                                tl.room_id
                            );
                            // For now, we paginate a cleared timeline in order to be able to show something at least.
                            // A proper solution would be what's described below, which would be to save a few event IDs
                            // and then either focus on them (if we're not close to the end of the timeline)
                            // or paginate backwards until we find them (only if we are close the end of the timeline).
                            should_continue_backwards_pagination = true;
                        }
                    }
                    if new_items.len() == tl.items.len() {
                        // println!("Timeline::handle_event(): no jump necessary for updated timeline of same length: {}", items.len());
                    } else if curr_first_id > new_items.len() {
                        println!(
                            "Timeline::handle_event(): jumping to bottom: curr_first_id {} is out of bounds for {} new items",
                            curr_first_id,
                            new_items.len()
                        );
                    } else if let Some((curr_item_idx, new_item_idx, new_item_scroll, _event_id)) =
                        find_new_item_matching_current_item(
                            0,
                            Some(0.0), // TODO replace
                            curr_first_id,
                            &tl.items,
                            &new_items,
                        )
                    {
                        if curr_item_idx != new_item_idx {
                            println!(
                                "Timeline::handle_event(): jumping view from event index {curr_item_idx} to new index {new_item_idx}, scroll {new_item_scroll}, event ID {_event_id}"
                            );
                            // portal_list.set_first_id_and_scroll(new_item_idx, new_item_scroll);
                            tl.prev_first_index = Some(new_item_idx);
                            // Set scrolled_past_read_marker false when we jump to a new event
                            tl.scrolled_past_read_marker = false;
                        }
                    }
                    //
                    // TODO: after an (un)ignore user event, all timelines are cleared. Handle that here.
                    //
                    else {
                        // eprintln!("!!! Couldn't find new event with matching ID for ANY event currently visible in the portal list");
                    }

                    if clear_cache {
                        tl.content_drawn_since_last_update.clear();
                        tl.profile_drawn_since_last_update.clear();
                        tl.fully_paginated = false;
                    } else {
                        tl.content_drawn_since_last_update
                            .remove(changed_indices.clone());
                        tl.profile_drawn_since_last_update
                            .remove(changed_indices.clone());
                        // println!("Timeline::handle_event(): changed_indices: {changed_indices:?}, items len: {}\ncontent drawn: {:#?}\nprofile drawn: {:#?}", items.len(), tl.content_drawn_since_last_update, tl.profile_drawn_since_last_update);
                    }
                    tl.items = new_items;
                    done_loading = true;
                }
                TimelineUpdate::NewUnreadMessagesCount(_unread_messages_count) => {
                    // jump_to_bottom.show_unread_message_badge(unread_messages_count);
                }
                TimelineUpdate::TargetEventFound {
                    target_event_id,
                    index,
                } => {
                    // println!("Target event found in room {}: {target_event_id}, index: {index}", tl.room_id);
                    tl.request_sender.send_if_modified(|requests| {
                        requests.retain(|r| r.room_id != tl.room_id);
                        // no need to notify/wake-up all receivers for a completed request
                        false
                    });

                    // sanity check: ensure the target event is in the timeline at the given `index`.
                    let item = tl.items.get(index);
                    let is_valid = item.is_some_and(|item| {
                        item.as_event()
                            .is_some_and(|ev| ev.event_id() == Some(&target_event_id))
                    });
                    // let loading_pane = self.view.loading_pane(id!(loading_pane));

                    // println!("TargetEventFound: is_valid? {is_valid}. room {}, event {target_event_id}, index {index} of {}\n  --> item: {item:?}", tl.room_id, tl.items.len());
                    if is_valid {
                    } else {
                        // Here, the target event was not found in the current timeline,
                        // or we found it previously but it is no longer in the timeline (or has moved),
                        // which means we encountered an error and are unable to jump to the target event.
                        eprintln!(
                            "Target event index {index} of {} is out of bounds for room {}",
                            tl.items.len(),
                            tl.room_id
                        );
                        // Show this error in the loading pane, which should already be open.
                        // loading_pane.set_state(LoadingPaneState::Error(String::from(
                        //     "Unable to find related message; it may have been deleted.",
                        // )));
                    }

                    should_continue_backwards_pagination = false;
                }
                TimelineUpdate::PaginationRunning(direction) => {
                    println!(
                        "Pagination running in room {} in {direction} direction",
                        tl.room_id
                    );
                    if direction == PaginationDirection::Backwards {
                        // top_space.set_visible(cx, true);
                        done_loading = false;
                    } else {
                        eprintln!("Unexpected PaginationRunning update in the Forwards direction");
                    }
                }
                TimelineUpdate::PaginationError { error, direction } => {
                    eprintln!(
                        "Pagination error ({direction}) in room {}: {error:?}",
                        tl.room_id
                    );
                    done_loading = true;
                }
                TimelineUpdate::PaginationIdle {
                    fully_paginated,
                    direction,
                } => {
                    if direction == PaginationDirection::Backwards {
                        // Don't set `done_loading` to `true`` here, because we want to keep the top space visible
                        // (with the "loading" message) until the corresponding `NewItems` update is received.
                        tl.fully_paginated = fully_paginated;
                        if fully_paginated {
                            done_loading = true;
                        }
                    } else {
                        eprintln!("Unexpected PaginationIdle update in the Forwards direction");
                    }
                }
                TimelineUpdate::EventDetailsFetched { event_id, result } => {
                    if let Err(_e) = result {
                        eprintln!(
                            "Failed to fetch details fetched for event {event_id} in room {}. Error: {_e:?}",
                            tl.room_id
                        );
                    }
                    // Here, to be most efficient, we could redraw only the updated event,
                    // but for now we just fall through and let the final `redraw()` call re-draw the whole timeline view.
                }
                TimelineUpdate::RoomMembersSynced => {
                    // println!("Timeline::handle_event(): room members fetched for room {}", tl.room_id);
                    // Here, to be most efficient, we could redraw only the user avatars and names in the timeline,
                    // but for now we just fall through and let the final `redraw()` call re-draw the whole timeline view.
                }
                TimelineUpdate::RoomMembersListFetched { members } => {
                    println!("RoomMembers list fetched !");
                    members.iter().for_each(|member| {
                        self.members.insert(
                            member.user_id().to_owned(),
                            FrontendRoomMember {
                                name: member.name().to_string(),
                                display_name_ambiguous: member.name_ambiguous(),
                                is_ignored: member.is_ignored(),
                                max_power_level: member.normalized_power_level(),
                            },
                        );
                    });
                    println!("{:?}", self.members);
                }
                TimelineUpdate::MediaFetched => {
                    println!(
                        "Timeline::handle_event(): media fetched for room {}",
                        tl.room_id
                    );
                    // Here, to be most efficient, we could redraw only the media items in the timeline,
                    // but for now we just fall through and let the final `redraw()` call re-draw the whole timeline view.
                }
                TimelineUpdate::MessageEdited {
                    timeline_event_id: _,
                    result: _,
                } => {
                    // self.view
                    //     .editing_pane(id!(editing_pane))
                    //     .handle_edit_result(cx, timeline_event_id, result);
                }
                TimelineUpdate::TypingUsers { users } => {
                    // This update loop should be kept tight & fast, so all we do here is
                    // save the list of typing users for future use after the loop exits.
                    // Then, we "process" it later (by turning it into a string) after the
                    // update loop has completed, which avoids unnecessary expensive work
                    // if the list of typing users gets updated many times in a row.
                    typing_users = users;
                }

                TimelineUpdate::UserPowerLevels(user_power_level) => {
                    tl.user_power = user_power_level;

                    // Update the visibility of the message input bar based on the new power levels.
                    let _can_send_message = user_power_level.can_send_message();
                    // self.view
                    //     .view(id!(input_bar))
                    //     .set_visible(cx, can_send_message);
                    // self.view
                    //     .view(id!(can_not_send_message_notice))
                    //     .set_visible(cx, !can_send_message);
                }

                TimelineUpdate::OwnUserReadReceipt(receipt) => {
                    tl.latest_own_user_receipt = Some(receipt);
                }
            }
        }

        if should_continue_backwards_pagination {
            println!("Continuing backwards pagination...");
            submit_async_request(MatrixRequest::PaginateRoomTimeline {
                room_id: tl.room_id.clone(),
                num_events: 50,
                direction: PaginationDirection::Backwards,
            });
        }

        if done_loading {
            // top_space.set_visible(cx, false);
        }

        if !typing_users.is_empty() {
            let _typing_notice_text = match typing_users.as_slice() {
                [] => String::new(),
                [user] => format!("{user} is typing "),
                [user1, user2] => format!("{user1} and {user2} are typing "),
                [user1, user2, others @ ..] => {
                    if others.len() > 1 {
                        format!("{user1}, {user2}, and {} are typing ", &others[0])
                    } else {
                        format!("{user1}, {user2}, and {} others are typing ", others.len())
                    }
                }
            };
            // Set the typing notice text and make its view visible.
            // self.view
            //     .label(id!(typing_label))
            //     .set_text(cx, &typing_notice_text);
            // self.view.view(id!(typing_notice)).set_visible(cx, true);
            // // Animate in the typing notice view (sliding it up from the bottom).
            // self.animator_play(cx, id!(typing_notice_animator.show));
            // // Start the typing notice text animation of bouncing dots.
            // self.view
            //     .typing_animation(id!(typing_animation))
            //     .start_animation(cx);
        } else {
            // Animate out the typing notice view (sliding it out towards the bottom).
            // self.animator_play(cx, id!(typing_notice_animator.hide));
            // self.view
            //     .typing_animation(id!(typing_animation))
            //     .stop_animation(cx);
        }

        if num_updates > 0 {
            // println!("Applied {} timeline updates for room {}, redrawing with {} items...", num_updates, tl.room_id, tl.items.len());
            self.update_frontend_state();
        }
    }

    /// Invoke this when this timeline is being shown,
    /// e.g., when the user navigates to this timeline.
    pub fn show_timeline(&mut self) {
        let room_id = self.room_id.clone();
        // just an optional sanity check
        assert!(
            self.tl_state.is_none(),
            "BUG: tried to show_timeline() into a timeline with existing state. \
            Did you forget to save the timeline state back to the global map of states?",
        );

        // Obtain the current user's power levels for this room.
        submit_async_request(MatrixRequest::GetRoomPowerLevels {
            room_id: room_id.clone(),
        });

        let state_opt = TIMELINE_STATES.lock().unwrap().remove(&room_id);
        let (tl_state, first_time_showing_room) = if let Some(existing) = state_opt {
            (existing, false)
        } else {
            let (_update_sender, update_receiver, request_sender) =
                take_timeline_endpoints(&room_id)
                    .expect("BUG: couldn't get timeline state for first-viewed room.");
            let new_tl_state = TimelineUiState {
                room_id: room_id.clone(),
                // We assume the user has all power levels by default, just to avoid
                // unexpectedly hiding any UI elements that should be visible to the user.
                // This doesn't mean that the user can actually perform all actions.
                user_power: UserPowerLevels::all(),
                // We assume timelines being viewed for the first time haven't been fully paginated.
                fully_paginated: false,
                items: Vector::new(),
                content_drawn_since_last_update: RangeSet::new(),
                profile_drawn_since_last_update: RangeSet::new(),
                update_receiver,
                request_sender,
                last_scrolled_index: usize::MAX,
                prev_first_index: None,
                scrolled_past_read_marker: false,
                latest_own_user_receipt: None,
            };
            (new_tl_state, true)
        };

        // Subscribe to typing notices, but hide the typing notice view initially.
        // self.view(id!(typing_notice)).set_visible(cx, false);
        submit_async_request(MatrixRequest::SubscribeToTypingNotices {
            room_id: room_id.clone(),
            subscribe: true,
        });

        submit_async_request(MatrixRequest::SubscribeToOwnUserReadReceiptsChanged {
            room_id: room_id.clone(),
            subscribe: true,
        });
        // Kick off a back pagination request for this room. This is "urgent",
        // because we want to show the user some messages as soon as possible
        // when they first open the room, and there might not be any messages yet.
        if first_time_showing_room && !tl_state.fully_paginated {
            println!(
                "Sending a first-time backwards pagination request for room {}",
                room_id
            );
            submit_async_request(MatrixRequest::PaginateRoomTimeline {
                room_id: room_id.clone(),
                num_events: 50,
                direction: PaginationDirection::Backwards,
            });
        }

        // This fetches the room members of the displayed timeline.
        submit_async_request(MatrixRequest::SyncRoomMemberList {
            room_id: room_id.clone(),
        });

        // As the final step, store the tl_state for this room into this RoomScreen widget,
        // such that it can be accessed in future event/draw handlers.
        self.tl_state = Some(tl_state);

        // Now that we have restored the TimelineUiState into this RoomScreen widget,
        // we can proceed to processing pending background updates, and if any were processed,
        // the timeline will also be redrawn.
        if first_time_showing_room {
            // let portal_list = self.portal_list(id!(list));
            self.process_timeline_updates();
        }

        self.update_frontend_state();
    }

    /// Invoke this when this RoomScreen/timeline is being hidden or no longer being shown.
    fn hide_timeline(&mut self) {
        self.save_state();

        // When closing a room view, we do the following with non-persistent states:
        // * Unsubscribe from typing notices, since we don't care about them
        //   when a given room isn't visible.
        // * Clear the location preview. We don't save this to the TimelineUiState
        //   because the location might change by the next time the user opens this same room.
        // self.location_preview(id!(location_preview)).clear();
        submit_async_request(MatrixRequest::SubscribeToTypingNotices {
            room_id: self.room_id.clone(),
            subscribe: false,
        });
        submit_async_request(MatrixRequest::SubscribeToOwnUserReadReceiptsChanged {
            room_id: self.room_id.clone(),
            subscribe: false,
        });
    }

    /// Removes the current room's visual UI state from this widget
    /// and saves it to the map of `TIMELINE_STATES` such that it can be restored later.
    ///
    /// Note: after calling this function, the widget's `tl_state` will be `None`.
    fn save_state(&mut self) {
        let Some(tl) = self.tl_state.take() else {
            eprintln!(
                "Timeline::save_state(): skipping due to missing state, room {:?}",
                self.room_id
            );
            return;
        };
        // Store this Timeline's `TimelineUiState` in the global map of states.
        TIMELINE_STATES
            .lock()
            .unwrap()
            .insert(tl.room_id.clone(), tl);
    }

    /// Sets this `RoomScreen` widget to display the timeline for the given room.
    pub fn set_displayed_room<S: Into<Option<String>>>(
        &mut self,
        room_id: OwnedRoomId,
        room_name: S,
    ) {
        // If the room is already being displayed, then do nothing.
        // if self.room_id.as_ref().is_some_and(|id| id == &room_id) {
        //     return;
        // }

        self.hide_timeline();
        // Reset the the state of the inner loading pane.
        // self.loading_pane(id!(loading_pane)).take_state();
        self.room_name = room_name_or_id(room_name.into(), &room_id);
        self.room_id = room_id.clone();

        // Clear any mention input state
        // let input_bar = self.view.room_input_bar(id!(input_bar));
        // let message_input = input_bar.mentionable_text_input(id!(message_input));
        // message_input.set_room_id(room_id);

        self.show_timeline();
    }

    /// Sends read receipts based on the current scroll position of the timeline.
    fn _send_user_read_receipts_based_on_scroll_pos(
        &mut self,
        _scrolled: bool,
        _first_id: usize,
        _visible_items: usize,
    ) {
        // TODO: leave this to frontend
    }

    /// Sends a backwards pagination request if the user is scrolling up
    /// and is approaching the top of the timeline.
    fn _send_pagination_request_based_on_scroll_pos(&mut self, _scrolled: bool, _first_id: usize) {
        // TODO: leave this to frontend
    }
}

/// Returns info about the item in the list of `new_items` that matches the event ID
/// of a visible item in the given `curr_items` list.
///
/// This info includes a tuple of:
/// 1. the index of the item in the current items list,
/// 2. the index of the item in the new items list,
/// 3. the positional "scroll" offset of the corresponding current item in the portal list,
/// 4. the unique event ID of the item.
fn find_new_item_matching_current_item(
    visible_items: usize,          // DUMMY PARAM TODO CHANGE THIS
    position_of_item: Option<f64>, // DUMMY PARAM TODO CHANGE THIS
    starting_at_curr_idx: usize,
    curr_items: &Vector<Arc<TimelineItem>>,
    new_items: &Vector<Arc<TimelineItem>>,
) -> Option<(usize, usize, f64, OwnedEventId)> {
    let mut curr_item_focus = curr_items.focus();
    let mut idx_curr = starting_at_curr_idx;
    let mut curr_items_with_ids: Vec<(usize, OwnedEventId)> = Vec::with_capacity(visible_items);

    // Find all items with real event IDs that are currently visible in the portal list.
    // TODO: if this is slow, we could limit it to 3-5 events at the most.
    if curr_items_with_ids.len() <= visible_items {
        while let Some(curr_item) = curr_item_focus.get(idx_curr) {
            if let Some(event_id) = curr_item.as_event().and_then(|ev| ev.event_id()) {
                curr_items_with_ids.push((idx_curr, event_id.to_owned()));
            }
            if curr_items_with_ids.len() >= visible_items {
                break;
            }
            idx_curr += 1;
        }
    }

    // Find a new item that has the same real event ID as any of the current items.
    for (idx_new, new_item) in new_items.iter().enumerate() {
        let Some(event_id) = new_item.as_event().and_then(|ev| ev.event_id()) else {
            continue;
        };
        if let Some((idx_curr, _)) = curr_items_with_ids
            .iter()
            .find(|(_, ev_id)| ev_id == event_id)
        {
            // Not all items in the portal list are guaranteed to have a position offset,
            // some may be zeroed-out, so we need to account for that possibility by only
            // using events that have a real non-zero area
            if let Some(pos_offset) = position_of_item {
                println!(
                    "Found matching event ID {event_id} at index {idx_new} in new items list, corresponding to current item index {idx_curr} at pos offset {pos_offset}"
                );
                return Some((*idx_curr, idx_new, pos_offset, event_id.to_owned()));
            }
        }
    }

    None
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontendRoomMember {
    name: String,
    max_power_level: i64,
    display_name_ambiguous: bool,
    is_ignored: bool,
}
