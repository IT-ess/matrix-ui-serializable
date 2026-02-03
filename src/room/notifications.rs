// Common imports
use crate::{
    init::singletons::{UIUpdateMessage, broadcast_event, get_event_bridge},
    models::events::{EmitEvent, ToastNotificationRequest},
};
use crossbeam_queue::SegQueue;
use matrix_sdk::Client;

// Platform imports
#[cfg(any(target_os = "android", target_os = "ios"))]
use anyhow::anyhow;
#[cfg(any(target_os = "android", target_os = "ios"))]
use matrix_sdk::ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use matrix_sdk::{
    Room,
    notification_settings::{NotificationSettings, RoomNotificationMode},
    ruma::{MilliSecondsSinceUnixEpoch, events::AnySyncTimelineEvent, serde::Raw},
};
#[cfg(any(target_os = "android", target_os = "ios"))]
use serde_json::{Map, json};
#[cfg(any(target_os = "android", target_os = "ios"))]
use url::Url;

//
// TOAST Notifications (in app)
//

static TOAST_NOTIFICATION: SegQueue<ToastNotificationRequest> = SegQueue::new();

/// Displays a new toast notification with the given message.
///
/// Toast notifications will be shown in the order they were enqueued.
pub fn enqueue_toast_notification(notification: ToastNotificationRequest) {
    TOAST_NOTIFICATION.push(notification);
    broadcast_event(UIUpdateMessage::RefreshUI);
}

pub async fn process_toast_notifications() -> anyhow::Result<()> {
    if TOAST_NOTIFICATION.is_empty() {
        return Ok(());
    }
    let event_bridge = get_event_bridge()?;
    while let Some(notif) = TOAST_NOTIFICATION.pop() {
        event_bridge.emit(EmitEvent::ToastNotification(notif));
    }
    Ok(())
}

//
// OS Notifications (push notifications for mobiles)
//

/// For user_language: The preferred language for receiving notifications (e.g. ‘en’ or ‘en-US’).
#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn register_mobile_push_notifications(
    client: &Client,
    token: String,
    user_language: String,
    android_sygnal_url: Url,
    ios_sygnal_url: Url,
    app_id: String,
) -> anyhow::Result<()> {
    let http_pusher = get_http_pusher(user_language.clone(), android_sygnal_url, ios_sygnal_url);
    let pusher_ids = PusherIds::new(token, app_id);

    let device_display_name = client
        .encryption()
        .get_own_device()
        .await?
        .ok_or(anyhow!("cannot get own device"))?
        .display_name()
        .unwrap_or("APP_NAME")
        .to_owned();
    let pusher = PusherInit {
        ids: pusher_ids,
        app_display_name: "MATRIX_SVELTE_CLIENT".to_string(),
        device_display_name,
        profile_tag: None,
        kind: PusherKind::Http(http_pusher),
        lang: user_language,
    };

    let pusher: Pusher = pusher.into();

    client.pusher().set(pusher).await?;
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn get_http_pusher(
    user_language: String,
    android_sygnal_url: Url,
    ios_sygnal_url: Url,
) -> matrix_sdk::ruma::push::HttpPusherData {
    // Due to Sygnal limitations (one instance cannot handle both FCM and APNS,
    // the gateway differs on iOS and Android)
    #[cfg(target_os = "ios")]
    let mut http_pusher = matrix_sdk::ruma::push::HttpPusherData::new(ios_sygnal_url.to_string());
    #[cfg(target_os = "android")]
    let mut http_pusher =
        matrix_sdk::ruma::push::HttpPusherData::new(android_sygnal_url.to_string());

    http_pusher.format = Some(matrix_sdk::ruma::push::PushFormat::EventIdOnly);

    // For iOS we define here the content of the notification.
    // For android, it is defined server-side.
    if cfg!(target_os = "ios") {
        // Poor localization of the alert, to be improved
        let title = if user_language.eq("fr") {
            "Nouveau message"
        } else {
            "New message"
        };
        let body = if user_language.eq("fr") {
            "Appuyez pour voir le message"
        } else {
            "Tap to view message"
        };

        let default_payload = json!( {
          "aps": {
              "mutable-content": 1,
              "content-available": 1,
              "alert": {
                  "title": title,
                  "body": body
              }
          }
        });
        let mut pusher_data = Map::new();
        pusher_data.insert("default_payload".to_owned(), default_payload);
        http_pusher.data = pusher_data;
    }

    http_pusher
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn register_os_desktop_notifications(client: &Client) {
    use std::time::SystemTime;

    use matrix_sdk::{ruma::MilliSecondsSinceUnixEpoch, sync::Notification};

    let server_settings = client.notification_settings().await;
    let Some(startup_ts) = MilliSecondsSinceUnixEpoch::from_system_time(SystemTime::now()) else {
        return;
    };

    client
        .register_notification_handler(
            move |notification: Notification, room: Room, client: Client| {
                let server_settings = server_settings.clone();
                async move {
                    use matrix_sdk::{
                        deserialized_responses::RawAnySyncOrStrippedTimelineEvent,
                        notification_settings::RoomNotificationMode,
                    };

                    let mode = global_or_room_mode(&server_settings, &room).await;
                    if mode == RoomNotificationMode::Mute {
                        return;
                    }

                    match notification.event {
                        RawAnySyncOrStrippedTimelineEvent::Sync(e) => {
                            match parse_full_notification(e, room, true).await {
                                Ok((summary, body, server_ts)) => {
                                    use crate::models::events::OsNotificationRequest;

                                    if server_ts < startup_ts {
                                        return;
                                    }

                                    if is_missing_mention(&body, mode, &client) {
                                        return;
                                    }

                                    let event_bridge =
                                        get_event_bridge().expect("Event bridge is not init");

                                    event_bridge.emit(EmitEvent::OsNotification(
                                        OsNotificationRequest::new(summary, body),
                                    ));
                                }
                                Err(err) => {
                                    use tracing::warn;

                                    warn!("Failed to extract notification data: {err}")
                                }
                            }
                        }
                        // Stripped events may be dropped silently because they're
                        // only relevant if we're not in a room, and we presumably
                        // don't want notifications for rooms we're not in.
                        RawAnySyncOrStrippedTimelineEvent::Stripped(_) => (),
                    }
                }
            },
        )
        .await;
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn global_or_room_mode(
    settings: &NotificationSettings,
    room: &Room,
) -> RoomNotificationMode {
    use matrix_sdk::notification_settings::{IsEncrypted, IsOneToOne};

    let room_mode = settings
        .get_user_defined_room_notification_mode(room.room_id())
        .await;
    if let Some(mode) = room_mode {
        return mode;
    }
    let is_one_to_one = match room.is_direct().await {
        Ok(true) => IsOneToOne::Yes,
        _ => IsOneToOne::No,
    };
    let is_encrypted = match room.encryption_state().is_encrypted() {
        true => IsEncrypted::Yes,
        false => IsEncrypted::No,
    };
    settings
        .get_default_room_notification_mode(is_encrypted, is_one_to_one)
        .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn is_missing_mention(body: &Option<String>, mode: RoomNotificationMode, client: &Client) -> bool {
    if let Some(body) = body
        && mode == RoomNotificationMode::MentionsAndKeywordsOnly
    {
        let mentioned = match client.user_id() {
            Some(user_id) => body.contains(user_id.localpart()),
            _ => false,
        };
        return !mentioned;
    }
    false
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn parse_full_notification(
    event: Raw<AnySyncTimelineEvent>,
    room: Room,
    show_body: bool,
) -> anyhow::Result<(String, Option<String>, MilliSecondsSinceUnixEpoch)> {
    let event = event.deserialize().map_err(anyhow::Error::from)?;

    let server_ts = event.origin_server_ts();

    let sender_id = event.sender();
    let sender = room
        .get_member_no_sync(sender_id)
        .await
        .map_err(anyhow::Error::from)?;

    let sender_name = sender
        .as_ref()
        .and_then(|m| m.display_name())
        .unwrap_or_else(|| sender_id.localpart());

    let summary = if let Some(room_name) = room.cached_display_name() {
        if room.is_direct().await.map_err(anyhow::Error::from)?
            && sender_name == room_name.to_string()
        {
            sender_name.to_owned()
        } else {
            format!("{sender_name} in {room_name}")
        }
    } else {
        sender_name.to_owned()
    };

    let body = if show_body {
        event_notification_body(&event, sender_name).map(truncate)
    } else {
        None
    };

    Ok((summary, body, server_ts))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn event_notification_body(event: &AnySyncTimelineEvent, sender_name: &str) -> Option<String> {
    use matrix_sdk::ruma::events::AnyMessageLikeEventContent;

    let AnySyncTimelineEvent::MessageLike(event) = event else {
        return None;
    };

    match event.original_content()? {
        AnyMessageLikeEventContent::RoomMessage(message) => {
            use matrix_sdk::ruma::events::room::message::MessageType;

            let body = match message.msgtype {
                MessageType::Audio(_) => {
                    format!("{sender_name} sent an audio file.")
                }
                MessageType::Emote(content) => content.body,
                MessageType::File(_) => {
                    format!("{sender_name} sent a file.")
                }
                MessageType::Image(_) => {
                    format!("{sender_name} sent an image.")
                }
                MessageType::Location(_) => {
                    format!("{sender_name} sent their location.")
                }
                MessageType::Notice(content) => content.body,
                MessageType::ServerNotice(content) => content.body,
                MessageType::Text(content) => content.body,
                MessageType::Video(_) => {
                    format!("{sender_name} sent a video.")
                }
                MessageType::VerificationRequest(_) => {
                    format!("{sender_name} sent a verification request.")
                }
                _ => {
                    format!("[Unknown message type: {:?}]", &message.msgtype)
                }
            };
            Some(body)
        }
        AnyMessageLikeEventContent::Sticker(_) => Some(format!("{sender_name} sent a sticker.")),
        _ => None,
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn truncate(s: String) -> String {
    use unicode_segmentation::UnicodeSegmentation;

    static MAX_LENGTH: usize = 5000;
    if s.graphemes(true).count() > MAX_LENGTH {
        let truncated: String = s.graphemes(true).take(MAX_LENGTH).collect();
        truncated + "..."
    } else {
        s
    }
}
