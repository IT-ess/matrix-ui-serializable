use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedRoomId};
use serde::{Deserialize, Serialize};

// Listen to events
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ListenEvent {
    RoomCreated,
    VerificationResult,
    MatrixUpdateCurrentActiveRoom,
    MatrixLogin,
    CancelVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixVerificationResponse {
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRoomStoreCreatedRequest {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixUpdateCurrentActiveRoom {
    pub room_id: OwnedRoomId,
    pub room_name: String,
}

/// The user's account credentials to create a new Matrix session
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixLoginPayload {
    pub username: String,
    pub password: String,
    pub homeserver_url: String,
    pub client_name: String,
}

// Emit events

#[derive(Debug, Clone)]
pub enum EmitEvent {
    RoomCreate(MatrixRoomStoreCreateRequest),
    VerificationStart(MatrixVerificationEmojis),
    ToastNotification(ToastNotificationRequest),
    OsNotification(OsNotificationRequest),
    OAuthUrl(String),
    ResetCrossSigngingUrl(String),
    NewlyCreatedRoomId(OwnedRoomId),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixVerificationEmojis {
    emojis: String,
}

impl MatrixVerificationEmojis {
    pub fn new(emojis: String) -> Self {
        Self { emojis }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRoomStoreCreateRequest {
    id: String,
}

impl MatrixRoomStoreCreateRequest {
    pub fn new(id: String) -> Self {
        Self { id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToastNotificationRequest {
    message: String,
    description: Option<String>,
    variant: ToastNotificationVariant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ToastNotificationVariant {
    Default,
    Description,
    Success,
    Info,
    Warning,
    Error,
}

impl ToastNotificationRequest {
    pub fn new(
        message: String,
        description: Option<String>,
        variant: ToastNotificationVariant,
    ) -> Self {
        if description.is_some() {
            // If there is a description, force the description variant.
            Self {
                message,
                description,
                variant: ToastNotificationVariant::Description,
            }
        } else {
            Self {
                message,
                description: None,
                variant,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsNotificationRequest {
    pub summary: String,
    pub body: Option<String>,
}

impl OsNotificationRequest {
    pub fn new(summary: String, body: Option<String>) -> Self {
        Self { summary, body }
    }
}

// Channel events

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum MediaStreamEvent {
    Started,
    Chunk {
        data: Vec<u8>,
        chunk_size: usize,
        bytes_received: usize,
    },
    Finished {
        total_bytes: usize,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum VerifyDeviceEvent {
    Requested,
    Done,
    Cancelled { reason: String },
}

// Commands
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDevice {
    pub device_id: OwnedDeviceId,
    pub is_verified: bool,
    pub is_verified_with_cross_signing: bool,
    pub display_name: Option<String>,
    pub last_seen_ts: Option<MilliSecondsSinceUnixEpoch>,
    pub guessed_type: DeviceGuessedType,
    pub is_current_device: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DeviceGuessedType {
    Android,
    Ios,
    Web,
    Desktop,
    Unknown,
}
