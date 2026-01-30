//! All the actions exposed to the frontend that returns a `Result`.

use crate::{
    FrontendVerificationState,
    init::{
        login::build_client,
        singletons::{APP_DATA_DIR, CLIENT, TEMP_CLIENT_SESSION, get_event_bridge},
    },
    models::{
        async_requests::MatrixRequest,
        events::{EmitEvent, FrontendDevice},
        misc::{EditRoomInformationPayload, EditUserInformationPayload},
        state_updater::StateUpdater,
    },
    room::rooms_list::{RoomsListUpdate, enqueue_rooms_list_update},
    utils::guess_device_type,
};
use anyhow::anyhow;
use mime::Mime;
use std::sync::Arc;
use tracing::info;

pub use crate::{init::FrontendAuthTypeResponse, models::events::VerifyDeviceEvent};
pub use matrix_sdk::ruma::{
    MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedUserId, UInt, UserId,
};
use matrix_sdk::{
    encryption::CrossSigningResetAuthType,
    ruma::{
        DeviceId, OwnedMxcUri,
        api::client::uiaa::{self, UserIdentifier},
    },
};

use tokio::sync::oneshot;

/// Try to build the client from the url given by the frontend and set its singleton.
/// Once called, the client is set and the init process can proceed (by calling check_homeserver_auth_type)
pub async fn build_client_from_homeserver_url(homeserver: String) -> crate::Result<()> {
    let (client, client_session) = build_client(Some(homeserver), None).await?;
    CLIENT.set(client).expect("Client already set");
    TEMP_CLIENT_SESSION
        .set(client_session)
        .expect("Client session already set");
    Ok(())
}

pub async fn check_homeserver_auth_type() -> crate::Result<FrontendAuthTypeResponse> {
    crate::init::check_homeserver_auth_type()
        .await
        .map_err(|e| e.into())
}

/// Submit a request to the Matrix Client that will be executed asynchronously.
pub fn submit_async_request(request: MatrixRequest) -> crate::Result<()> {
    crate::models::async_requests::submit_async_request(request);
    Ok(())
}

/// Fetches a given User Profile and adds it to this lib's User Profile cache.
pub async fn fetch_user_profile(
    user_id: OwnedUserId,
    room_id: Option<OwnedRoomId>,
) -> crate::Result<bool> {
    Ok(crate::user::user_profile::fetch_user_profile(user_id, room_id).await)
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
    client.logout().await?;
    // clear the db folder
    std::fs::remove_dir_all(APP_DATA_DIR.get().unwrap().clone().join("surreal"))
        .map_err(|e| e.into())
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
    match CLIENT.get() {
        Some(c) => c.is_active(),
        None => false,
    }
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
                    UserIdentifier::UserIdOrLocalpart(client.user_id().unwrap().to_string()),
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

pub async fn register_notifications(_token: String, _user_language: String) -> anyhow::Result<()> {
    let client = CLIENT.wait();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    crate::room::notifications::register_mobile_push_notifications(&client, _token, _user_language)
        .await?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    crate::room::notifications::register_os_desktop_notifications(client).await;

    Ok(())
}
