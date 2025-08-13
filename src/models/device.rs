use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedDeviceId};
use serde::Serialize;

use crate::utils::DeviceGuessedType;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDevice {
    pub device_id: OwnedDeviceId,
    pub is_verified: bool,
    pub is_verified_with_cross_signing: bool,
    pub display_name: Option<String>,
    pub registration_date: MilliSecondsSinceUnixEpoch,
    pub guessed_type: DeviceGuessedType,
    pub is_current_device: bool,
}
