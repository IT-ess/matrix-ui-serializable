//! Serializable result types for the background push-notification decryption
//! flow (see [`crate::commands::get_notification_item`]).
//!
//! These mirror [`matrix_sdk_ui::notification_client::NotificationStatus`] /
//! `NotificationItem`, but only keep the fields a frontend needs to render an
//! OS notification, and are `Serialize`/`Deserialize` so a native (no-webview)
//! caller can pass them around freely.

use matrix_sdk::ruma::OwnedEventId;
use serde::{Deserialize, Serialize};

/// The outcome of resolving a single push notification.
///
/// Mirrors [`matrix_sdk_ui::notification_client::NotificationStatus`] so the
/// caller can decide whether to display the notification, suppress it, or show
/// a generic placeholder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FrontendNotificationStatus {
    /// The event was found and decrypted; display this notification.
    Event(FrontendNotificationItem),
    /// The event couldn't be found by the network queries.
    NotFound,
    /// The event was filtered out by the user's push rules or because its
    /// sender is ignored; do not notify.
    FilteredOut,
    /// The event was redacted and has no meaningful content.
    Redacted,
}

/// The full result returned by [`crate::commands::get_notification_item`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendNotificationResult {
    /// The resolved notification status.
    pub status: FrontendNotificationStatus,
    /// A re-serialized `FullMatrixSession` when the access/refresh tokens were
    /// rotated while resolving the notification. When `Some`, persist it to the
    /// same storage the main app reads so both processes stay in sync.
    /// `None` when the tokens were unchanged.
    pub refreshed_session: Option<String>,
}

/// The decrypted, display-ready content of a single notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendNotificationItem {
    /// Title line, e.g. the sender name, or `"{sender} in {room}"` for
    /// non-DM rooms.
    pub summary: String,
    /// Notification body (message preview). `None` when the event has no
    /// displayable textual content.
    pub body: Option<String>,
    /// Display name of the sender, if known.
    pub sender_display_name: Option<String>,
    /// MXC URL of the sender's avatar, if any.
    pub sender_avatar_url: Option<String>,
    /// Computed display name of the room.
    pub room_display_name: String,
    /// MXC URL of the room's avatar, if any.
    pub room_avatar_url: Option<String>,
    /// Whether the room is a direct message.
    pub is_dm: bool,
    /// Whether the notification is "noisy" (a push action requests a sound).
    /// `None` when the push actions couldn't be determined.
    pub is_noisy: Option<bool>,
    /// Whether the event mentions/highlights the current user. `None` when the
    /// push actions couldn't be determined.
    pub has_mention: Option<bool>,
    /// The root event id of the thread this event belongs to, if any.
    pub thread_id: Option<OwnedEventId>,
}
