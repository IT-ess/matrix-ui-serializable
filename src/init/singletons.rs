use anyhow::anyhow;
use std::{path::PathBuf, sync::OnceLock};

use matrix_sdk::{Client, ruma::OwnedUserId};
use matrix_sdk_ui::sync_service::SyncService;
use tokio::sync::{
    broadcast,
    mpsc::{Receiver, UnboundedSender},
};

use crate::{
    init::session::ClientSession,
    models::{
        async_requests::MatrixRequest,
        event_bridge::EventBridge,
        events::{MatrixRoomStoreCreatedRequest, MatrixVerificationResponse},
    },
};

/// The sender used by [`submit_async_request`] to send requests to the async worker thread.
/// Currently there is only one, but it can be cloned if we need more concurrent senders.
pub static REQUEST_SENDER: OnceLock<UnboundedSender<MatrixRequest>> = OnceLock::new();

/// The singleton sync service.
pub static SYNC_SERVICE: OnceLock<SyncService> = OnceLock::new();

/// Flag set by `handle_rooms_loading_state` when all rooms are loaded.
/// if rooms have been synced or not.
pub static ALL_ROOMS_LOADED: OnceLock<bool> = OnceLock::new();

/// The temporary Client Session used during login
pub static TEMP_CLIENT_SESSION: OnceLock<ClientSession> = OnceLock::new();

/// The logged-in Matrix client, which can be freely and cheaply cloned.
pub static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn get_cloned_client() -> Option<Client> {
    CLIENT.get().cloned()
}

/// The current User's ID
pub static CURRENT_USER_ID: OnceLock<OwnedUserId> = OnceLock::new();

/// Flag to be set once the frontend Login Store is up and ready
pub static LOGIN_STORE_READY: OnceLock<bool> = OnceLock::new();

#[derive(Debug, Clone)]
pub enum UIUpdateMessage {
    RefreshUI,
}

// Global broadcaster instance
static GLOBAL_BROADCASTER: OnceLock<GlobalBroadcaster> = OnceLock::new();

pub struct GlobalBroadcaster {
    sender: broadcast::Sender<UIUpdateMessage>,
}

pub static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

impl GlobalBroadcaster {
    fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    fn broadcast(
        &self,
        message: UIUpdateMessage,
    ) -> Result<usize, broadcast::error::SendError<UIUpdateMessage>> {
        self.sender.send(message)
    }

    fn subscribe(&self) -> broadcast::Receiver<UIUpdateMessage> {
        self.sender.subscribe()
    }
}

// Initialize the global broadcaster (call this once at startup)
pub fn init_broadcaster(capacity: usize) -> Result<(), &'static str> {
    GLOBAL_BROADCASTER
        .set(GlobalBroadcaster::new(capacity))
        .map_err(|_| "Broadcaster already initialized")
}

// Globally available function to broadcast messages
pub fn broadcast_event(message: UIUpdateMessage) {
    let broadcaster = GLOBAL_BROADCASTER
        .get()
        .expect("Broadcaster not initialized. Call init_broadcaster() first.");

    let _ = broadcaster.broadcast(message);
}

// Globally available function to create receivers
pub fn subscribe_to_events() -> Result<broadcast::Receiver<UIUpdateMessage>, &'static str> {
    let broadcaster = GLOBAL_BROADCASTER
        .get()
        .ok_or("Broadcaster not initialized. Call init_broadcaster() first.")?;

    Ok(broadcaster.subscribe())
}

// Lib -> adapter communication

pub static EVENT_BRIDGE: OnceLock<EventBridge> = OnceLock::new();

pub fn get_event_bridge<'a>() -> anyhow::Result<&'a EventBridge> {
    let bridge = EVENT_BRIDGE
        .get()
        .ok_or(anyhow!("The event bridge is not yet set"))?;
    Ok(bridge)
}

// Adapter -> lib communication

pub static ROOM_CREATED_RECEIVER: OnceLock<
    tokio::sync::Mutex<Receiver<MatrixRoomStoreCreatedRequest>>,
> = OnceLock::new();

pub async fn get_room_created_receiver_lock<'a>()
-> anyhow::Result<tokio::sync::MutexGuard<'a, Receiver<MatrixRoomStoreCreatedRequest>>> {
    let recv = ROOM_CREATED_RECEIVER
        .get()
        .ok_or(anyhow!("The room created receiver is not yet set"))?;
    Ok(recv.lock().await)
}

pub static VERIFICATION_RESPONSE_RECEIVER: OnceLock<
    tokio::sync::Mutex<Receiver<MatrixVerificationResponse>>,
> = OnceLock::new();

pub async fn get_verification_response_receiver_lock<'a>()
-> anyhow::Result<tokio::sync::MutexGuard<'a, Receiver<MatrixVerificationResponse>>> {
    let recv = VERIFICATION_RESPONSE_RECEIVER
        .get()
        .ok_or(anyhow!("The verification response receiver is not yet set"))?;
    Ok(recv.lock().await)
}
