use std::time::Duration;

use matrix_sdk::{
    Client, Room, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch,
        events::{
            key::verification::request::ToDeviceKeyVerificationRequestEvent,
            room::{
                member::OriginalSyncRoomMemberEvent,
                message::{MessageType, OriginalSyncRoomMessageEvent},
            },
        },
    },
};
use matrix_sdk_ui::timeline::{LatestEventValue, Profile, TimelineDetails};
use tokio::runtime::Handle;
use tracing::warn;

use crate::init::singletons::MEMBERSHIP_UPDATES_EXPIRY_MAP;

use super::{
    emoji_verification::request_verification_handler, event_preview::text_preview_of_timeline_item,
};

// These event handlers handle only the verification events. Other events are managed by the matrix_sdk_ui sync service.
pub fn add_event_handlers(client: &Client) {
    client.add_event_handler(
        |ev: ToDeviceKeyVerificationRequestEvent, client: Client| async move {
            if let Some(request) = client
                .encryption()
                .get_verification_request(&ev.sender, &ev.content.transaction_id)
                .await
            {
                Handle::current().spawn(request_verification_handler(
                            client,
                            request,
                        ));
            }
            else {
                warn!("Skipping invalid verification request from {}, transaction ID: {}\n   Content: {:?}",
                    ev.sender, ev.content.transaction_id, ev.content,
                );
            }
        },
    );

    client.add_event_handler(
        |ev: OriginalSyncRoomMessageEvent, client: Client| async move {
            if let MessageType::VerificationRequest(_) = &ev.content.msgtype {
                if let Some(request) = client
                    .encryption()
                    .get_verification_request(&ev.sender, &ev.event_id)
                    .await
                {
                    Handle::current().spawn(request_verification_handler(
                                client,
                                request,
                            ));
                }
                else {
                    warn!("Skipping invalid verification request from {}, event ID: {}\n   Content: {:?}",
                        ev.sender, ev.event_id, ev.content,
                    );
                }
            }
        }
    );

    client.add_event_handler(|_: OriginalSyncRoomMemberEvent, room: Room| async move {
        if room.state() != RoomState::Joined {
            return;
        }
        // We add this room in an expiry map. When the duration will be expired, it will trigger
        // a request to update memberships on this room. Since it is a map, new values overwrite
        // the previous "Instant" and then acts as a debouncer.
        // The members are updated from cache only, so we don't try to fetch large member lists
        // from too large rooms we may haven't seen yet.
        MEMBERSHIP_UPDATES_EXPIRY_MAP.insert(room.room_id().to_owned(), Duration::from_millis(200));
    });
}

/// Returns the timestamp and text preview of the given `latest_event` timeline item.
///
/// If the sender profile of the event is not yet available, this function will
/// generate a preview using the sender's user ID instead of their display name,
/// and will submit a background async request to fetch the details for this event.
pub fn get_latest_event_details(
    latest_event: LatestEventValue,
) -> Option<(MilliSecondsSinceUnixEpoch, String)> {
    match latest_event {
        LatestEventValue::Local {
            timestamp,
            sender: _,
            profile,
            content,
            state: _,
        } => {
            let sender_username =
                get_sender_username_from_profile(profile).unwrap_or("".to_owned());
            Some((
                timestamp,
                text_preview_of_timeline_item(&content, &sender_username)
                    .format_with(&sender_username, true),
            ))
        }
        LatestEventValue::Remote {
            timestamp,
            sender: _,
            is_own: _,
            profile,
            content,
        } => {
            let sender_username =
                get_sender_username_from_profile(profile).unwrap_or("".to_owned());
            Some((
                timestamp,
                text_preview_of_timeline_item(&content, &sender_username)
                    .format_with(&sender_username, true),
            ))
        }
        LatestEventValue::RemoteInvite {
            timestamp,
            inviter: _,
            inviter_profile,
        } => {
            let sender_username =
                get_sender_username_from_profile(inviter_profile).unwrap_or("".to_owned());
            Some((timestamp, format!("New invite from {sender_username}")))
        }
        LatestEventValue::None => None,
    }
}

// TODO: use timeline details directly or fetch them
pub(crate) fn get_sender_username_from_profile(
    profile: TimelineDetails<Profile>,
) -> Option<String> {
    match profile {
        TimelineDetails::Ready(p) => p.display_name,
        _ => None,
    }
}
