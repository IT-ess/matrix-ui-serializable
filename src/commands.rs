//! All the actions exposed to the frontend that returns a `Result`.

use crate::{
    FrontendVerificationState, UserProfile,
    events::timeline::TimelineKind,
    get_timeline_kind,
    init::{
        login::build_client,
        singletons::{
            CLIENT, CURRENT_USER_ID, HAS_SESSION_STORED, TEMP_CLIENT, TEMP_CLIENT_SESSION,
            get_event_bridge,
        },
    },
    models::{
        async_requests::MatrixRequest,
        events::{EmitEvent, FrontendDevice},
        matrix_uri::{MatrixUriIntent, get_matrix_uri_intent, parse_address},
        misc::{EditRoomInformationPayload, EditUserInformationPayload},
        state_updater::StateUpdater,
    },
    room::{
        frontend_events::events_dto::{FrontendTimelineItem, map_event_timeline_item},
        joined_room::get_timeline,
        preview::{CachedRoomPreview, get_or_fetch_room_preview},
        rooms_list::{RoomsListUpdate, enqueue_rooms_list_update},
    },
    user::{user_power_level::UserPowerLevels, user_profile::with_user_profile},
    utils::guess_device_type,
};
use anyhow::anyhow;
use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource};
use mime::Mime;
use rand::{RngExt, distr::Alphanumeric, rng};
use std::{sync::Arc, time::Duration};
use tracing::{error, info};
use url::Url;

pub use crate::room::preview::SerializableRoomPreview;
pub use crate::{
    init::FrontendAuthTypeResponse, models::events::VerifyDeviceEvent,
    models::matrix_uri::MatrixUriPillInfo,
};
pub use matrix_sdk::ruma::{
    MatrixToUri, MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedEventId, OwnedRoomId,
    OwnedServerName, OwnedUserId, UInt, UserId,
};
use matrix_sdk::{
    attachment::{AttachmentInfo, Thumbnail},
    encryption::CrossSigningResetAuthType,
    media::MediaRequestParameters,
    ruma::{
        DeviceId, OwnedMxcUri, OwnedRoomOrAliasId,
        api::client::uiaa::{self, MatrixUserIdentifier, UserIdentifier},
        events::room::{MediaSource, message::TextMessageEventContent},
    },
};

use tokio::{sync::oneshot, time::sleep};

/// Try to build the client from the url given by the frontend and set its singleton.
/// Once called, the client is set and the init process can proceed (by calling check_homeserver_auth_type)
pub async fn build_temp_client_from_homeserver_url(homeserver: String) -> crate::Result<()> {
    let (client, client_session) = build_client(Some(homeserver), None, None).await?;
    {
        let mut temp_session = TEMP_CLIENT_SESSION.lock().unwrap();
        *temp_session = Some(client_session);
    }
    {
        let mut guard = TEMP_CLIENT.lock.lock().unwrap();
        *guard = Some(client);
    }
    TEMP_CLIENT.cvar.notify_all();
    Ok(())
}

pub async fn check_homeserver_auth_type() -> crate::Result<FrontendAuthTypeResponse> {
    let (auth_type, _) = crate::init::check_homeserver_auth_type().await?;
    Ok(auth_type)
}

/// Submit a request to the Matrix Client that will be executed asynchronously.
pub fn submit_async_request(request: MatrixRequest) {
    crate::models::async_requests::submit_async_request(request);
}

/// Polls the UserProfile cache to get a profile or fetch it if needed.
/// It timeouts after 8 secs.
pub async fn fetch_user_profile(
    user_id: OwnedUserId,
    room_id: Option<&OwnedRoomId>,
) -> crate::Result<UserProfile> {
    // Poll the cache every 200ms, up to 40 times (8 seconds timeout)
    for _ in 0..40 {
        let user_profile_opt =
            with_user_profile(user_id.clone(), room_id, true, |profile, _| profile.clone());

        if let Some(user_profile) = user_profile_opt {
            return Ok(user_profile);
        }

        sleep(Duration::from_millis(200)).await;
    }

    Err(crate::Error::Anyhow(anyhow!(
        "Timed out waiting for user profile to populate"
    )))
}

/// Get the list of this user's account registered devices.
pub async fn get_devices(user_id: &UserId) -> crate::Result<Vec<FrontendDevice>> {
    let client = CLIENT.wait();
    let devices_list = client.devices().await.map_err(anyhow::Error::from)?;
    let devices: Vec<FrontendDevice> = client
        .encryption()
        .get_user_devices(user_id)
        .await?
        .devices()
        .filter(|device| !device.is_deleted())
        .map(|device| {
            let last_seen_ts = devices_list
                .devices
                .iter()
                .find(|i| i.device_id.eq(device.device_id()))
                .and_then(|d| d.last_seen_ts);
            FrontendDevice {
                device_id: device.device_id().to_owned(),
                display_name: device.display_name().map(|n| n.to_owned()),
                is_verified: device.is_verified(),
                is_verified_with_cross_signing: device.is_verified_with_cross_signing(),
                last_seen_ts,
                guessed_type: guess_device_type(device.display_name()),
                is_current_device: device.device_id().eq(client.device_id().unwrap()),
            }
        })
        .collect();
    Ok(devices)
}

/// Check whether this device is verified or not
pub fn check_device_verification() -> FrontendVerificationState {
    match CLIENT.get() {
        Some(client) => client.encryption().verification_state().get().into(),
        None => FrontendVerificationState::new(matrix_sdk::encryption::VerificationState::Unknown),
    }
}

/// Checks whether this account has secret backup setup
pub async fn has_backup_setup() -> crate::Result<bool> {
    crate::account::backup::has_backup_setup().await
}

/// Try to restore encryption state from backup
pub async fn restore_backup_with_passphrase(passphrase: String) -> crate::Result<()> {
    crate::account::backup::restore_backup_with_passphrase(passphrase).await
}

/// Setup a new backup for secret keys
pub async fn setup_new_backup() -> crate::Result<String> {
    crate::account::backup::setup_new_backup().await
}

pub fn get_dm_room_from_user_id(user_id: &UserId) -> crate::Result<Option<OwnedRoomId>> {
    let client = CLIENT.wait();
    Ok(client.get_dm_room(user_id).map(|r| r.room_id().to_owned()))
}

/// Start the SAS V1 Emoji verification process with another user's device.
pub async fn verify_device(
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    cancel_rx: oneshot::Receiver<()>,
    status_tx: std::sync::mpsc::Sender<VerifyDeviceEvent>,
) -> crate::Result<()> {
    crate::events::emoji_verification::verify_device(&user_id, &device_id, cancel_rx, status_tx)
        .await
        .map_err(crate::Error::Anyhow)
}

/// Disconnect the connected user
pub async fn disconnect_user() -> crate::Result<()> {
    let client = CLIENT.wait();
    // Logout the session
    client.logout().await.map_err(|e| e.into())
}

/// Disconnect the connected user
pub async fn check_if_last_device() -> crate::Result<bool> {
    let client = CLIENT.wait();
    client
        .encryption()
        .recovery()
        .is_last_device()
        .await
        .map_err(anyhow::Error::from)
        .map_err(|e| e.into())
}

/// Check the login state
pub fn is_logged_in() -> bool {
    CLIENT.get().is_some()
}

pub fn has_session_stored() -> bool {
    *HAS_SESSION_STORED.wait()
}

pub async fn reset_cross_signing(password: Option<String>) -> crate::Result<()> {
    let client = CLIENT.wait();
    let encryption = client.encryption();
    if let Some(handle) = encryption
        .recovery()
        .reset_identity()
        .await
        .map_err(anyhow::Error::from)?
    {
        match handle.auth_type() {
            CrossSigningResetAuthType::Uiaa(uiaa) => {
                if password.is_none() {
                    panic!("You should provide a password if you reset identity in Uiaa mode");
                }
                let mut password = uiaa::Password::new(
                    UserIdentifier::Matrix(MatrixUserIdentifier::new(
                        client.user_id().unwrap().to_string(),
                    )),
                    password.unwrap(),
                );
                password.session = uiaa.session.clone();

                handle
                    .reset(Some(uiaa::AuthData::Password(password)))
                    .await
                    .map_err(anyhow::Error::from)?;
            }
            CrossSigningResetAuthType::OAuth(o) => {
                let url = o.approval_url.clone();
                info!(
                    "To reset your end-to-end encryption cross-signing identity, \
                    you first need to approve it at {}",
                    url
                );
                tokio::spawn(async move { handle.reset(None).await });
                let event_bridge = get_event_bridge().expect("event bridge should be defined");
                event_bridge.emit(EmitEvent::ResetCrossSigngingUrl(url.to_string()));
            }
        }
    }
    Ok(())
}

pub async fn edit_user_information(
    payload: EditUserInformationPayload,
    updater: Arc<Box<dyn StateUpdater>>,
) -> crate::Result<()> {
    let client = CLIENT.wait();
    let account_manager = client.account();
    if let Some(ref display_name) = payload.new_display_name {
        account_manager.set_display_name(Some(display_name)).await?;
    }
    if let Some(ref mxc_uri) = payload.new_avatar_uri {
        account_manager.set_avatar_url(Some(mxc_uri)).await?;
    }
    if let Some(ref device_name) = payload.new_device_name {
        rename_device(client.device_id().unwrap(), device_name).await?;
    }
    updater.update_current_user_info(
        None,
        payload.new_avatar_uri,
        payload.new_display_name,
        payload.new_device_name,
    )?;
    Ok(())
}

pub async fn rename_device(device_id: &DeviceId, display_name: &str) -> crate::Result<()> {
    let client = CLIENT.wait();
    match client.rename_device(device_id, display_name).await {
        Ok(_) => Ok(()),
        Err(err) => Err(crate::Error::Anyhow(anyhow!(err))),
    }
}

pub async fn upload_media(content_type: Mime, data: Vec<u8>) -> crate::Result<OwnedMxcUri> {
    let client = CLIENT.wait();
    let res = client.media().upload(&content_type, data, None).await?;
    Ok(res.content_uri)
}

pub fn filter_room_list(keywords: String) {
    enqueue_rooms_list_update(RoomsListUpdate::ApplyFilter { keywords });
}

pub async fn define_room_informations(payload: EditRoomInformationPayload) -> crate::Result<()> {
    let client = CLIENT.wait();
    let room = client
        .get_room(&payload.room_id)
        .ok_or(anyhow!("Couldn't get room for given id"))?;
    if let Some(uri) = payload.new_avatar_uri {
        room.set_avatar_url(&uri, None).await?;
    }
    if let Some(name) = payload.new_display_name {
        room.set_name(name).await?;
    }
    if let Some(topic) = payload.topic {
        room.set_room_topic(&topic).await?;
    }
    Ok(())
}

pub fn get_dm_room_id_or_create_it(user_id: OwnedUserId) -> Option<OwnedRoomId> {
    let client = CLIENT.wait();
    let res = client
        .get_dm_room(&user_id)
        .map(|room| room.room_id().to_owned());
    if res.is_none() {
        // If the room doesn't exist, then we send a request to create it.
        // The room_id will be sent to front through an event.
        crate::models::async_requests::submit_async_request(MatrixRequest::CreateDMRoom {
            user_id,
        });
    }
    res
}

pub async fn get_event_from_main_timeline(
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> crate::Result<FrontendTimelineItem> {
    let kind = TimelineKind::MainRoom { room_id };
    let timeline = get_timeline(&kind).ok_or(anyhow!("Cannot get timeline"))?;

    let pl = timeline.room().power_levels_or_default().await;

    let event = timeline
        .item_by_event_id(&event_id)
        .await
        .ok_or(anyhow!("Event not found"))?;

    let unique_id: String = rng()
        .sample_iter(Alphanumeric)
        .take(7)
        .map(char::from)
        .collect();

    Ok(map_event_timeline_item(
        unique_id,
        &event,
        &kind,
        &UserPowerLevels::from(&pl, CURRENT_USER_ID.get().unwrap()),
    )
    .ok_or(anyhow!("This item cannot be mapped to a frontend struct"))?)
}

#[allow(clippy::too_many_arguments)]
pub async fn send_media_message(
    room_id: OwnedRoomId,
    thread_root: Option<OwnedEventId>,
    buffer: Vec<u8>,
    filename: String,
    mime_type: Mime,
    caption: Option<String>,
    in_reply_to: Option<OwnedEventId>,
    info: AttachmentInfo,
    thumbnail: Option<Thumbnail>,
) -> crate::Result<()> {
    let timeline = get_timeline(&get_timeline_kind(room_id, thread_root))
        .ok_or(anyhow!("Cannot get timeline"))?;

    let source = AttachmentSource::Data {
        bytes: buffer,
        filename,
    };

    let config = AttachmentConfig {
        caption: caption.map(TextMessageEventContent::plain),
        in_reply_to,
        info: Some(info),
        thumbnail,
        ..Default::default()
    };

    timeline
        .send_attachment(source, mime_type, config)
        .await
        .map_err(anyhow::Error::from)
        .map_err(Into::into)
}

/// Fetches the full preview information for the given non parsed address.
/// Also fetches that room preview's avatar, if it had an avatar URL.
pub async fn try_get_room_preview_from_address(
    text: &str,
) -> anyhow::Result<(SerializableRoomPreview, Vec<OwnedServerName>)> {
    let (room, via) = parse_address(text)?;
    poll_room_preview(room, via).await
}

async fn poll_room_preview(
    room: OwnedRoomOrAliasId,
    via: Vec<OwnedServerName>,
) -> anyhow::Result<(SerializableRoomPreview, Vec<OwnedServerName>)> {
    // Poll the cache every 100ms, up to 40 times (4 seconds timeout)
    for _ in 0..40 {
        let preview_opt = get_or_fetch_room_preview(&room, &via);

        if let CachedRoomPreview::Loaded { preview } = preview_opt {
            return Ok((preview, via));
        }

        sleep(Duration::from_millis(100)).await;
    }

    Err(anyhow!("Timed out while waiting for room preview"))
}

/// Handler for the matrix: URIs. It will send a Tauri event to the frontend with the required data.
pub fn handle_matrix_uri(uri: &Url) {
    if let Ok(bridge) = get_event_bridge()
        && let Ok(intent) = get_matrix_uri_intent(uri.as_str())
    {
        bridge.emit(EmitEvent::MatrixUriIntent(intent));
    } else {
        error!("Cannot translate URI to local intent");
    }
}

pub async fn fetch_matrix_pill_info(uri: &str) -> anyhow::Result<MatrixUriPillInfo> {
    let intent = get_matrix_uri_intent(uri)?;
    match intent {
        MatrixUriIntent::Room((room, via, event_opt)) => {
            let (room_preview, via) = poll_room_preview(room, via).await?;
            Ok(MatrixUriPillInfo::Room((room_preview, via, event_opt)))
        }
        MatrixUriIntent::User(user_id) => Ok(MatrixUriPillInfo::User(with_user_profile(
            user_id,
            None,
            true,
            |profile, _| profile.clone(),
        ))),
    }
}

pub async fn get_matrix_to_permalink_for_room(room_id: OwnedRoomId) -> anyhow::Result<MatrixToUri> {
    let client = CLIENT.get().ok_or(anyhow!("Client not available"))?;
    let room = client.get_room(&room_id).ok_or(anyhow!("Room not found"))?;
    room.matrix_to_permalink().await.map_err(Into::into)
}

pub async fn register_notifications(
    _token: String,
    _user_language: String,
    _android_sygnal_url: Url,
    _ios_sygnal_url: Url,
    _app_id: String,
) -> anyhow::Result<()> {
    let client = CLIENT.wait();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    crate::room::notifications::register_mobile_push_notifications(
        &client,
        _token,
        _user_language,
        _android_sygnal_url,
        _ios_sygnal_url,
        _app_id,
    )
    .await?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    crate::room::notifications::register_os_desktop_notifications(client).await;

    Ok(())
}

/// Resolve and decrypt a single push notification from a background process.
///
/// This is meant to be called from a native, UI-less entry point (e.g. an iOS
/// Notification Service Extension or an Android background service) when a
/// silent push containing only a `room_id` and `event_id` is received, possibly
/// while the main app is killed. It is fully self-contained: it does **not**
/// rely on [`crate::init`], the global singletons, the sync service, the
/// `StateUpdater` or the event bridge.
///
/// It restores a lightweight client from the stored session (using a distinct
/// cross-process store lock holder so it can run alongside a live main app),
/// then uses [`matrix_sdk_ui::notification_client::NotificationClient`] to fetch
/// and decrypt the event, returning display-ready content for the caller to turn
/// into an OS notification.
///
/// * `session` — the serialized `FullMatrixSession` stored by the adapter (the
///   same string passed to `LibConfig`).
/// * `app_data_dir` — the application data directory (same as `LibConfig`).
///
/// If the access/refresh tokens were rotated while resolving the notification,
/// the returned [`FrontendNotificationResult::refreshed_session`] holds an
/// updated serialized session that the caller must persist.
// #[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn get_notification_item(
    session: String,
    app_data_dir: std::path::PathBuf,
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> crate::Result<crate::models::notification::FrontendNotificationResult> {
    use crate::{
        init::session::FullMatrixSession,
        init::singletons::APP_DATA_DIR,
        models::notification::{
            FrontendNotificationItem, FrontendNotificationResult, FrontendNotificationStatus,
        },
        room::notifications::{event_notification_body, truncate},
    };
    use matrix_sdk_ui::notification_client::{
        NotificationClient, NotificationEvent, NotificationProcessSetup, NotificationStatus,
    };

    // The background process is fresh, so APP_DATA_DIR is unset; `build_client`
    // waits on it. Ignore the error in case it was already set.
    let _ = APP_DATA_DIR.set(app_data_dir);

    let FullMatrixSession {
        client_session,
        user_session,
    } = serde_json::from_str(&session).map_err(anyhow::Error::from)?;

    // Build a client sharing the same on-disk store as the main app, but with a
    // distinct cross-process lock holder name. Do not set the CLIENT singleton
    // or start any sync/worker: this client is local to this call.
    let (client, client_session) =
        build_client(None, Some(client_session), Some("notifications")).await?;
    client.restore_session(user_session).await?;

    // Watch for token rotation during the notification fetch so we can hand the
    // refreshed session back to the caller for persistence.
    let mut session_changes = client.subscribe_to_session_changes();

    let notification_client =
        NotificationClient::new(client.clone(), NotificationProcessSetup::MultipleProcesses)
            .await
            .map_err(anyhow::Error::from)?;

    let status = notification_client
        .get_notification(&room_id, &event_id)
        .await
        .map_err(anyhow::Error::from)?;

    let status = match status {
        NotificationStatus::Event(item) => {
            let item = *item;
            let sender_name = item
                .sender_display_name
                .clone()
                .unwrap_or_else(|| item.event.sender().localpart().to_owned());

            let body = match &item.event {
                NotificationEvent::Timeline(event) => {
                    event_notification_body(event, &sender_name).map(truncate)
                }
                NotificationEvent::Invite(_) => Some(format!("{sender_name} invited you to chat.")),
            };

            let summary = if item.is_direct_message_room {
                sender_name
            } else {
                format!("{sender_name} in {}", item.room_computed_display_name)
            };

            let sender_avatar = if let Some(mxc_uri) = item.sender_avatar_url {
                client
                    .media()
                    .get_media_content(
                        &MediaRequestParameters {
                            source: MediaSource::Plain(OwnedMxcUri::from(mxc_uri)),
                            format: matrix_sdk::media::MediaFormat::File,
                        },
                        true,
                    )
                    .await
                    .ok() // We ignore the error
            } else {
                None
            };

            FrontendNotificationStatus::Event(FrontendNotificationItem {
                summary,
                body,
                sender_display_name: item.sender_display_name,
                sender_avatar,
                room_display_name: item.room_computed_display_name,
                room_avatar_url: item.room_avatar_url,
                is_dm: item.is_direct_message_room,
                is_noisy: item.is_noisy,
                has_mention: item.has_mention,
                thread_id: item.thread_id,
            })
        }
        NotificationStatus::EventNotFound => FrontendNotificationStatus::NotFound,
        NotificationStatus::EventFilteredOut => FrontendNotificationStatus::FilteredOut,
        NotificationStatus::EventRedacted => FrontendNotificationStatus::Redacted,
    };

    // Drain any session change that happened during the fetch.
    let mut tokens_refreshed = false;
    while let Ok(change) = session_changes.try_recv() {
        if matches!(change, matrix_sdk::SessionChange::TokensRefreshed) {
            tokens_refreshed = true;
        }
    }

    let refreshed_session = if tokens_refreshed {
        client
            .session()
            .map(|auth| serde_json::to_string(&FullMatrixSession::new(client_session, auth)))
            .transpose()
            .map_err(anyhow::Error::from)?
    } else {
        None
    };

    Ok(FrontendNotificationResult {
        status,
        refreshed_session,
    })
}
