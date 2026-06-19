use matrix_sdk::ruma::{OwnedMxcUri, OwnedRoomId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Payload to edit current user's information.
/// Only the Some(...) fields are updated, None are ignored.
pub struct EditUserInformationPayload {
    pub new_display_name: Option<String>,
    pub new_avatar_uri: Option<OwnedMxcUri>,
    pub new_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Payload to edit current user's information.
/// Only the Some(...) fields are updated, None are ignored.
pub struct EditRoomInformationPayload {
    pub room_id: OwnedRoomId,
    pub new_display_name: Option<String>,
    pub new_avatar_uri: Option<OwnedMxcUri>,
    pub topic: Option<String>,
}
