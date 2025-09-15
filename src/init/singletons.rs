use anyhow::anyhow;
use seshat::Database;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use matrix_sdk::{Client, ruma::OwnedRoomId};
use matrix_sdk_ui::sync_service::SyncService;
use tokio::sync::{
    broadcast,
    mpsc::{self, UnboundedSender},
};

use crate::{
    models::{
        async_requests::MatrixRequest,
        event_bridge::EventBridge,
        events::{MatrixRoomStoreCreatedRequest, MatrixVerificationResponse},
    },
    room::joined_room::JoinedRoomDetails,
};

/// The sender used by [`submit_async_request`] to send requests to the async worker thread.
/// Currently there is only one, but it can be cloned if we need more concurrent senders.
pub static REQUEST_SENDER: OnceLock<UnboundedSender<MatrixRequest>> = OnceLock::new();

/// The singleton sync service.
pub static SYNC_SERVICE: OnceLock<SyncService> = OnceLock::new();

pub fn _get_sync_service() -> Option<&'static SyncService> {
    SYNC_SERVICE.get()
}

/// Information about all joined rooms that our client currently know about.
pub static ALL_JOINED_ROOMS: Mutex<BTreeMap<OwnedRoomId, JoinedRoomDetails>> =
    Mutex::new(BTreeMap::new());

pub static TOMBSTONED_ROOMS: Mutex<BTreeMap<OwnedRoomId, OwnedRoomId>> =
    Mutex::new(BTreeMap::new());

pub static LOG_ROOM_LIST_DIFFS: bool = true;

pub static LOG_TIMELINE_DIFFS: bool = true;

/// The logged-in Matrix client, which can be freely and cheaply cloned.
pub static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn get_client() -> Option<Client> {
    CLIENT.get().cloned()
}

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
pub fn broadcast_event(message: UIUpdateMessage) -> Result<usize, Box<dyn std::error::Error>> {
    let broadcaster = GLOBAL_BROADCASTER
        .get()
        .ok_or("Broadcaster not initialized. Call init_broadcaster() first.")?;

    Ok(broadcaster.broadcast(message)?)
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
    tokio::sync::Mutex<mpsc::Receiver<MatrixRoomStoreCreatedRequest>>,
> = OnceLock::new();

pub async fn get_room_created_receiver_lock<'a>() -> anyhow::Result<
    tokio::sync::MutexGuard<'a, tokio::sync::mpsc::Receiver<MatrixRoomStoreCreatedRequest>>,
> {
    let recv = ROOM_CREATED_RECEIVER
        .get()
        .ok_or(anyhow!("The room created receiver is not yet set"))?;
    Ok(recv.lock().await)
}

pub static VERIFICATION_RESPONSE_RECEIVER: OnceLock<
    tokio::sync::Mutex<mpsc::Receiver<MatrixVerificationResponse>>,
> = OnceLock::new();

pub async fn get_verification_response_receiver_lock<'a>() -> anyhow::Result<
    tokio::sync::MutexGuard<'a, tokio::sync::mpsc::Receiver<MatrixVerificationResponse>>,
> {
    let recv = VERIFICATION_RESPONSE_RECEIVER
        .get()
        .ok_or(anyhow!("The verification response receiver is not yet set"))?;
    Ok(recv.lock().await)
}

// Seshat

pub static SESHAT_DATABASE: OnceLock<Arc<Mutex<Database>>> = OnceLock::new();

pub fn get_seshat_db_lock() -> Option<Arc<Mutex<Database>>> {
    SESHAT_DATABASE.get().cloned()
}
