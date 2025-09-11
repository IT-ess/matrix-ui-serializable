use std::{path::PathBuf, sync::Arc, thread, time::Duration};

use serde::{Serialize, ser::Serializer};
use tokio::{runtime::Handle, sync::broadcast};

use crate::{
    init::{
        session::try_restore_session_to_state,
        singletons::{
            CLIENT, EVENT_BRIDGE, REQUEST_SENDER, ROOM_CREATED_RECEIVER, TEMP_DIR,
            VERIFICATION_RESPONSE_RECEIVER,
        },
        workers::{async_main_loop, async_worker},
    },
    models::{
        event_bridge::EventBridge,
        events::{
            EmitEvent, MatrixRoomStoreCreatedRequest, MatrixUpdateCurrentActiveRoom,
            MatrixVerificationResponse, ToastNotificationRequest, ToastNotificationVariant,
        },
        state_updater::StateUpdater,
    },
    room::{
        notifications::enqueue_toast_notification,
        rooms_list::{RoomsCollectionStatus, RoomsListUpdate, enqueue_rooms_list_update},
    },
};

pub mod commands;
pub(crate) mod events;
pub(crate) mod init;
pub mod models;
pub(crate) mod room;
pub(crate) mod seshat;
pub(crate) mod stores;
pub(crate) mod user;
pub(crate) mod utils;

pub type Result<T> = std::result::Result<T, Error>;

/// matrix-ui-serializable Error enum
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error(transparent)]
    MatrixSdk(#[from] matrix_sdk::Error),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

/// Required `mpsc:Receiver`s to listen to incoming events
pub struct EventReceivers {
    room_created_receiver: mpsc::Receiver<MatrixRoomStoreCreatedRequest>,
    verification_response_receiver: mpsc::Receiver<MatrixVerificationResponse>,
    room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
}

impl EventReceivers {
    pub fn new(
        room_created_receiver: mpsc::Receiver<MatrixRoomStoreCreatedRequest>,
        verification_response_receiver: mpsc::Receiver<MatrixVerificationResponse>,
        room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
    ) -> Self {
        Self {
            room_created_receiver,
            verification_response_receiver,
            room_update_receiver,
        }
    }
}

/// The required configuration for this lib. Adapters must implement updaters and event_receivers.
pub struct LibConfig {
    /// The functions that will be in charge of updating the frontend states / stores
    /// from the backend state
    updaters: Box<dyn StateUpdater>,
    /// To listen to events coming from the frontend, we use channels.
    event_receivers: EventReceivers,
    /// The session option stored (or not) by the adapter. It is serialized.
    session_option: Option<String>,
    /// The required configuration for mobile push notifications to work
    mobile_push_notifications_config: Option<MobilePushNotificationConfig>,
    /// A PathBuf to the temporary dir of the app
    temp_dir: PathBuf,
}

impl LibConfig {
    pub fn new(
        updaters: Box<dyn StateUpdater>,
        mobile_push_notifications_config: Option<MobilePushNotificationConfig>,
        event_receivers: EventReceivers,
        session_option: Option<String>,
        temp_dir: PathBuf,
    ) -> Self {
        Self {
            updaters,
            mobile_push_notifications_config,
            event_receivers,
            session_option,
            temp_dir,
        }
    }
}

/// Function to be called once your app is starting to init this lib.
/// This will start the workers and return a `Receiver` to forward outgoing events.
pub fn init(config: LibConfig) -> broadcast::Receiver<EmitEvent> {
    // Lib -> adapter events
    let (event_bridge, broadcast_receiver) = EventBridge::new();
    EVENT_BRIDGE
        .set(event_bridge)
        .expect("Couldn't set the event bridge");

    // Adapter -> lib events
    ROOM_CREATED_RECEIVER
        .set(tokio::sync::Mutex::new(
            config.event_receivers.room_created_receiver,
        ))
        .expect("Couldn't set room created receiver");

    VERIFICATION_RESPONSE_RECEIVER
        .set(tokio::sync::Mutex::new(
            config.event_receivers.verification_response_receiver,
        ))
        .expect("Couldn't set the verification response receiver");

    // Create a channel to be used between UI thread(s) and the async worker thread.
    crate::init::singletons::init_broadcaster(16).expect("Couldn't init the UI broadcaster"); // TODO: adapt capacity if needed

    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<MatrixRequest>();
    REQUEST_SENDER
        .set(sender)
        .expect("BUG: REQUEST_SENDER already set!");

    TEMP_DIR
        .set(config.temp_dir)
        .expect("Couldn't set temporary dir");

    // Wait for frontend to be ready before proceeding with the init.
    LOGIN_STORE_READY.wait();
    println!("FRONTEND IS READY");
    let _monitor = Handle::current().spawn(async move {
        // Init seshat first
        seshat::commands::init_event_index("password".to_string()) // TODO: add to config. This password is used for encryption only, that is deactivated for now anyway.
            .await
            .expect("Couldn't init seshat index");

        println!(
            "IS SESHAT EMPTY: {}",
            seshat::commands::is_event_index_empty().await.unwrap()
        );
        let db_stats = seshat::commands::get_stats().await.unwrap();
        println!("SESHAT EVENT COUNT: {}", db_stats.event_count);
        println!("SESHAT ROOM COUNT: {}", db_stats.room_count);

        let client = try_restore_session_to_state(
            config.session_option,
            config.mobile_push_notifications_config,
        )
        .await
        .expect("Couldn't try to restore session");

        let client = match client {
            Some(new_login) => {
                let _ = &config
                    .updaters
                    .update_login_state(
                        LoginState::Restored,
                        new_login
                            .user_id()
                            .map_or(None, |val| Some(val.to_string())),
                    )
                    .expect("Couldn't update login state");
                new_login
            }
            None => {
                println!("Waiting for login request...");
                thread::sleep(Duration::from_secs(3)); // Block the thread for 3 secs to let the frontend init itself.
                let _ = &config
                    .updaters
                    .update_login_state(LoginState::AwaitingForLogin, None)
                    .expect("Couldn't update login state");
                // We await frontend to call the login command and set the client
                // loop until client is set
                CLIENT.wait();
                let client = CLIENT.get().unwrap().clone();
                let _ = &config
                    .updaters
                    .update_login_state(
                        LoginState::LoggedIn,
                        client
                            .user_id()
                            .clone()
                            .map_or(None, |val| Some(val.to_string())),
                    )
                    .expect("Couldn't update login state");
                client
            }
        };

        let mut verification_subscriber = client.encryption().verification_state();

        let updaters_arc = Arc::new(config.updaters);
        let inner_updaters = updaters_arc.clone();
        tokio::task::spawn(async move {
            while let Some(state) = verification_subscriber.next().await {
                inner_updaters
                    .clone()
                    .update_verification_state(FrontendVerificationState::new(state))
                    .expect("Couldn't update verification state in Svelte Store");
            }
        });

        let mut ui_event_receiver =
            crate::init::singletons::subscribe_to_events().expect("Couldn't get UI event receiver"); // subscribe to events so the sender(s) never fail

        // Spawn the actual async worker thread.
        let mut worker_join_handle = Handle::current().spawn(async_worker(receiver));

        // // Start the main loop that drives the Matrix client SDK.
        let mut main_loop_join_handle = Handle::current().spawn(async_main_loop(
            client,
            updaters_arc,
            config.event_receivers.room_update_receiver,
        ));

        #[allow(clippy::never_loop)] // unsure if needed, just following tokio's examples.
        loop {
            tokio::select! {
                result = &mut main_loop_join_handle => {
                    match result {
                        Ok(Ok(())) => {
                            eprintln!("BUG: main async loop task ended unexpectedly!");
                        }
                        Ok(Err(e)) => {
                            eprintln!("Error: main async loop task ended:\n\t{e:?}");
                            enqueue_rooms_list_update(RoomsListUpdate::Status {
                                status: RoomsCollectionStatus::Error(e.to_string()),
                            });
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!("Rooms list update error: {e}"),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        },
                        Err(e) => {
                            eprintln!("BUG: failed to join main async loop task: {e:?}");
                        }
                    }
                    break;
                }
                result = &mut worker_join_handle => {
                    match result {
                        Ok(Ok(())) => {
                            eprintln!("BUG: async worker task ended unexpectedly!");
                        }
                        Ok(Err(e)) => {
                            eprintln!("Error: async worker task ended:\n\t{e:?}");
                            enqueue_rooms_list_update(RoomsListUpdate::Status {
                                status: RoomsCollectionStatus::Error(e.to_string()),
                            });
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                format!("Rooms list update error: {e}"),
                                None,
                                ToastNotificationVariant::Error,
                            ));
                        },
                        Err(e) => {
                            eprintln!("BUG: failed to join async worker task: {e:?}");
                        }
                    }
                    break;
                }
                _ = ui_event_receiver.recv() => {
                    #[cfg(debug_assertions)]
                    println!("Received UI update event");
                }
            }
        }
    });

    // Return broadcast receiver for the adapter to forward outgoing events
    broadcast_receiver
}

// Re-exports

pub use init::login::MatrixClientConfig;
pub use init::singletons::LOGIN_STORE_READY;
pub use models::async_requests::*;
pub use room::notifications::MobilePushNotificationConfig;
pub use room::room_screen::RoomScreen;
pub use room::rooms_list::RoomsList;
pub use stores::login_store::{FrontendSyncServiceState, FrontendVerificationState, LoginState};
pub use user::user_profile::UserProfileMap;

// The adapter needs some types in those modules
pub use matrix_sdk::media::MediaRequestParameters;
pub use matrix_sdk::ruma::{OwnedDeviceId, OwnedRoomId, OwnedUserId};
pub use tokio::sync::mpsc;
pub use tokio::sync::oneshot;
