use std::path::PathBuf;

use crate::{
    init::{
        login::{LoginRequest, MatrixClientConfig},
        singletons::get_client,
    },
    models::{async_requests::MatrixRequest, events::FrontendDevice},
    room::notifications::MobilePushNotificationConfig,
    utils::guess_device_type,
};
use matrix_sdk::ruma::{OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId};

pub async fn login_and_create_new_session(
    config: MatrixClientConfig,
    mobile_push_config: Option<MobilePushNotificationConfig>,
    app_data_dir: PathBuf,
) -> crate::Result<String> {
    crate::init::session::login_and_get_session(
        LoginRequest::LoginByPassword(config),
        mobile_push_config,
        app_data_dir,
    )
    .await
}

pub fn submit_async_request(request: MatrixRequest) -> crate::Result<()> {
    crate::models::async_requests::submit_async_request(request);
    Ok(())
}

pub async fn fetch_user_profile(
    user_id: OwnedUserId,
    room_id: Option<OwnedRoomId>,
) -> crate::Result<bool> {
    Ok(crate::user::user_profile::fetch_user_profile(user_id, room_id).await)
}

pub async fn get_devices(user_id: &UserId) -> crate::Result<Vec<FrontendDevice>> {
    let client = get_client().expect("Client should be defined at this state");
    let devices: Vec<FrontendDevice> = client
        .encryption()
        .get_user_devices(user_id)
        .await?
        .devices()
        .filter(|device| !device.is_deleted())
        .map(|device| FrontendDevice {
            device_id: device.device_id().to_owned(),
            display_name: device.display_name().map(|n| n.to_string()),
            is_verified: device.is_verified(),
            is_verified_with_cross_signing: device.is_verified_with_cross_signing(),
            registration_date: device.first_time_seen_ts(),
            guessed_type: guess_device_type(device.display_name()),
            is_current_device: device.device_id().eq(client.device_id().unwrap()),
        })
        .collect();
    Ok(devices)
}

pub async fn verify_device(user_id: OwnedUserId, device_id: OwnedDeviceId) -> crate::Result<()> {
    crate::events::emoji_verification::verify_device(&user_id, &device_id)
        .await
        .map_err(|e| crate::Error::Anyhow(e))
}
