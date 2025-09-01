use matrix_sdk::{
    Client, Room, RoomState,
    ruma::{
        MilliSecondsSinceUnixEpoch, OwnedRoomId,
        events::{
            key::verification::request::ToDeviceKeyVerificationRequestEvent,
            room::message::{MessageType, OriginalSyncRoomMessageEvent, SyncRoomMessageEvent},
        },
    },
};
use matrix_sdk_ui::timeline::EventTimelineItem;
use tokio::runtime::Handle;

use crate::{seshat::utils::sync_to_seshat_event, user::user_profile::get_user_profile_option};

use super::{
    emoji_verification::request_verification_handler, event_preview::text_preview_of_timeline_item,
};

// These event handlers handle only the verification events. Other events are managed by the matrix_sdk_ui sync service.
pub fn add_event_handlers(client: Client) -> anyhow::Result<Client> {
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
                eprintln!("Skipping invalid verification request from {}, transaction ID: {}\n   Content: {:?}",
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
                    eprintln!("Skipping invalid verification request from {}, event ID: {}\n   Content: {:?}",
                        ev.sender, ev.event_id, ev.content,
                    );
                }
            }
        }
    );

    client.add_event_handler(
        async move |ev: SyncRoomMessageEvent, room: Room, _client: Client| {
            if room.state() != RoomState::Joined {
                return;
            }
            let user_profile_option = get_user_profile_option(ev.sender()).await;

            let profile = if let Some(user_profile) = user_profile_option {
                seshat::Profile {
                    displayname: user_profile.username,
                    avatar_url: user_profile.avatar_url.map(|s| s.to_string()),
                }
            } else {
                seshat::Profile {
                    displayname: None,
                    avatar_url: None,
                }
            };

            let optional_event = sync_to_seshat_event(ev, room.room_id().to_owned());
            match optional_event {
                Some(event) => {
                    crate::seshat::commands::add_event_to_index(event, profile)
                        .await
                        .expect("Couldn't add event to seshat db");
                    println!("Event added to seshat db")
                }
                None => {
                    println!("Redacted event ignored.");
                    return;
                }
            }
        },
    );

    Ok(client)
}

/// Returns the timestamp and text preview of the given `latest_event` timeline item.
///
/// If the sender profile of the event is not yet available, this function will
/// generate a preview using the sender's user ID instead of their display name,
/// and will submit a background async request to fetch the details for this event.
pub fn get_latest_event_details(
    latest_event: &EventTimelineItem,
    room_id: &OwnedRoomId,
) -> (MilliSecondsSinceUnixEpoch, String) {
    #[cfg(debug_assertions)]
    println!("Formating event coming from: {:?}", latest_event.sender());

    let sender_username = crate::utils::get_or_fetch_event_sender(latest_event, Some(room_id));
    (
        latest_event.timestamp(),
        text_preview_of_timeline_item(latest_event.content(), &sender_username)
            .format_with(&sender_username, true),
    )
}
