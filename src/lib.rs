use std::{path::PathBuf, sync::Arc, thread, time::Duration};

use futures::StreamExt;
use serde::{Serialize, ser::Serializer};
use tokio::{
    runtime::Handle,
    sync::{broadcast, mpsc::unbounded_channel},
};
use tracing::{error, info};
use url::Url;

use crate::{
    init::{
        FrontendAuthTypeResponse, check_homeserver_auth_type,
        session::{setup_token_background_save, try_restore_session_to_state},
        singletons::{
            APP_DATA_DIR, CURRENT_USER_ID, EVENT_BRIDGE, REQUEST_SENDER, ROOM_CREATED_RECEIVER,
            VERIFICATION_RESPONSE_RECEIVER, get_cloned_client,
        },
        workers::{async_main_loop, async_worker},
    },
    models::{
        event_bridge::EventBridge,
        events::{
            EmitEvent, MatrixLoginPayload, MatrixRoomStoreCreatedRequest,
            MatrixUpdateCurrentActiveRoom, MatrixVerificationResponse, ToastNotificationRequest,
            ToastNotificationVariant,
        },
        state_updater::StateUpdater,
    },
    room::{
        notifications::enqueue_toast_notification,
        rooms_list::{RoomsCollectionStatus, RoomsListUpdate, enqueue_rooms_list_update},
    },
};

pub(crate) mod account;
pub mod commands;
pub(crate) mod events;
pub(crate) mod init;
pub mod models;
pub(crate) mod room;
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
    // Event based
    room_created_receiver: mpsc::Receiver<MatrixRoomStoreCreatedRequest>,
    verification_response_receiver: mpsc::Receiver<MatrixVerificationResponse>,
    room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
    // Command based
    matrix_login_receiver: mpsc::Receiver<MatrixLoginPayload>,
    oauth_deeplink_receiver: mpsc::Receiver<Url>,
}

impl EventReceivers {
    pub fn new(
        room_created_receiver: mpsc::Receiver<MatrixRoomStoreCreatedRequest>,
        verification_response_receiver: mpsc::Receiver<MatrixVerificationResponse>,
        room_update_receiver: mpsc::Receiver<MatrixUpdateCurrentActiveRoom>,
        matrix_login_receiver: mpsc::Receiver<MatrixLoginPayload>,
        oauth_deeplink_receiver: mpsc::Receiver<Url>,
    ) -> Self {
        Self {
            room_created_receiver,
            verification_response_receiver,
            room_update_receiver,
            matrix_login_receiver,
            oauth_deeplink_receiver,
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
    /// A PathBuf to the application data directory
    app_data_dir: PathBuf,
    /// A callback URL that will handle the redirection to your app when logging with the Oauth flow
    oauth_redirect_uri: Url,
}

impl LibConfig {
    pub fn new(
        updaters: Box<dyn StateUpdater>,
        event_receivers: EventReceivers,
        session_option: Option<String>,
        app_data_dir: PathBuf,
        oauth_redirect_uri: Url,
    ) -> Self {
        Self {
            updaters,
            event_receivers,
            session_option,
            app_data_dir,
            oauth_redirect_uri,
        }
    }
}

/// Function to be called once your app is starting to init this lib.
/// This will start the workers and return a `Receiver` to forward outgoing events.
pub async fn init(mut config: LibConfig) -> broadcast::Receiver<EmitEvent> {
    APP_DATA_DIR
        .set(config.app_data_dir)
        .expect("Couldn't set app data dir");

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
    init::singletons::init_broadcaster(16).expect("Couldn't init the UI broadcaster"); // TODO: adapt capacity if needed

    let (sender, receiver) = unbounded_channel::<MatrixRequest>();
    REQUEST_SENDER
        .set(sender)
        .expect("BUG: REQUEST_SENDER already set!");

    // Wait for frontend to be ready before proceeding with the init.
    LOGIN_STORE_READY.wait();
    info!("FRONTEND IS READY");

    let _monitor = Handle::current().spawn(async move {
        let updaters_arc = Arc::new(config.updaters);
        let inner_updaters = updaters_arc.clone();

        // Setup the token refresher thread
        setup_token_background_save(inner_updaters.clone());

        let client_opt = match try_restore_session_to_state(config.session_option).await {
            Ok(opt) => opt,
            Err(e) => {
                enqueue_toast_notification(ToastNotificationRequest::new(
                    format!("Failed to restore session, falling back on login. Error: {e}"),
                    None,
                    ToastNotificationVariant::Error,
                ));
                None
            }
        };

        let (client, _has_been_restored) = match client_opt {
            Some(restored) => {
                if let Err(e) = &inner_updaters.update_login_state(
                    LoginState::Restored,
                    restored.user_id().map(|v| v.to_string()),
                ) {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("Cannot update login state. Error: {e}"),
                        None,
                        ToastNotificationVariant::Error,
                    ))
                }
                (restored, true)
            }
            None => {
                // Block the thread for 1 sec to let the frontend init itself.
                // Note: that's a shitty way to handle a race condition, but otherwise the
                // Svelte Login store doesn't receive the updates.
                thread::sleep(Duration::from_secs(1));
                if let Err(e) =
                    &inner_updaters.update_login_state(LoginState::AwaitingForHomeserver, None)
                {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("Cannot update login state. Error: {e}"),
                        None,
                        ToastNotificationVariant::Error,
                    ))
                }
                info!("Waiting for homeserver selection...");

                let client = if let Ok(auth_type) = check_homeserver_auth_type().await {
                    let client =
                        get_cloned_client().expect("client should be defined at this point");
                    let serialized_session = match auth_type {
                        FrontendAuthTypeResponse::Oauth => init::oauth::register_and_login_oauth(
                            &client,
                            config.event_receivers.oauth_deeplink_receiver,
                            &config.oauth_redirect_uri,
                        )
                        .await
                        .expect("Failed to login with OAuth"),
                        FrontendAuthTypeResponse::Matrix => {
                            // wait for frontend payload
                            let login_payload = config
                                .event_receivers
                                .matrix_login_receiver
                                .recv()
                                .await
                                .expect("no login sender to listen to");
                            init::login::login_and_persist_matrix_session(
                                &client,
                                login_payload.username.clone(),
                                login_payload.password.clone(),
                                login_payload.client_name.clone(),
                            )
                            .await
                            .expect("Failed to login with Matrix Auth")
                        }
                        FrontendAuthTypeResponse::WrongUrl => {
                            enqueue_toast_notification(ToastNotificationRequest::new(
                                "The homeserver URL is incorrect".to_owned(),
                                Some("Please restart the app and try another one.".to_owned()),
                                ToastNotificationVariant::Error,
                            ));
                            "".to_owned()
                        }
                    };

                    if let Err(e) = &inner_updaters
                        .persist_login_session(serialized_session)
                        .await
                    {
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            format!("Failed to persist login session. Error: {e}"),
                            None,
                            ToastNotificationVariant::Error,
                        ));
                    }

                    client
                } else {
                    panic!("Unknown homeserver auth type")
                };

                // Update frontend login state
                if let Err(e) = &inner_updaters.update_login_state(
                    LoginState::LoggedIn,
                    client.user_id().map(|v| v.to_string()),
                ) {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("Cannot update login state. Error: {e}"),
                        None,
                        ToastNotificationVariant::Error,
                    ))
                }
                (client, false)
            }
        };

        CURRENT_USER_ID
            .set(client.user_id().unwrap().to_owned())
            .expect("Couldn't set CURRENT_USER_ID singleton");

        let user_avatar = client.account().get_avatar_url().await.map_or(None, |a| a);

        let user_display_name = client
            .account()
            .get_display_name()
            .await
            .map_or(None, |n| n);

        let device_name = client
            .encryption()
            .get_own_device()
            .await
            .ok()
            .flatten()
            .and_then(|d| d.display_name().map(|s| s.to_owned()));

        if let Err(e) = inner_updaters.update_current_user_info(
            Some(client.user_id().unwrap().to_owned()),
            user_avatar,
            user_display_name,
            device_name,
        ) {
            enqueue_toast_notification(ToastNotificationRequest::new(
                format!("Cannot update current user info. Error: {e}"),
                None,
                ToastNotificationVariant::Error,
            ))
        }

        let mut verification_subscriber = client.encryption().verification_state();

        let verification_state_updaters = inner_updaters.clone();
        tokio::task::spawn(async move {
            while let Some(state) = verification_subscriber.next().await {
                if let Err(e) = verification_state_updaters
                    .update_verification_state(FrontendVerificationState::new(state))
                {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("Cannot update verification store. Error: {e}"),
                        None,
                        ToastNotificationVariant::Error,
                    ))
                }
            }
        });

        let mut state_stream = client.encryption().recovery().state_stream();
        let recovery_state_updaters = inner_updaters.clone();
        tokio::task::spawn(async move {
            while let Some(update) = state_stream.next().await {
                recovery_state_updaters
                    .update_recovery_state(update)
                    .expect("couldn't update frontend recovery state")
            }
        });

        let mut ui_event_receiver =
            init::singletons::subscribe_to_events().expect("Couldn't get UI event receiver"); // subscribe to events so the sender(s) never fail

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
                            error!("BUG: main async loop task ended unexpectedly!");
                        }
                        Ok(Err(e)) => {
                            error!("Error: main async loop task ended:\n\t{e:?}");
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
                            error!("BUG: failed to join main async loop task: {e:?}");
                        }
                    }
                    break;
                }
                result = &mut worker_join_handle => {
                    match result {
                        Ok(Ok(())) => {
                            error!("BUG: async worker task ended unexpectedly!");
                        }
                        Ok(Err(e)) => {
                            error!("Error: async worker task ended:\n\t{e:?}");
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
                            error!("BUG: failed to join async worker task: {e:?}");
                        }
                    }
                    break;
                }
                _ = ui_event_receiver.recv() => {
                    #[cfg(debug_assertions)]
                    tracing::trace!("Received UI update event");
                }
            }
        }
    });

    // Return broadcast receiver for the adapter to forward outgoing events
    broadcast_receiver
}

// Re-exports

pub use init::session::FullMatrixSession;
pub use init::singletons::LOGIN_STORE_READY;
pub use models::async_requests::*;
pub use room::room_screen::RoomScreen;
pub use room::rooms_list::RoomsList;
pub use stores::login_store::{FrontendSyncServiceState, FrontendVerificationState, LoginState};
pub use user::user_profile::UserProfileMap;

// The adapter needs some types in those modules
pub use matrix_sdk::AuthSession;
pub use matrix_sdk::encryption::recovery::RecoveryState;
pub use matrix_sdk::media::MediaRequestParameters;
pub use matrix_sdk::ruma::{OwnedDeviceId, OwnedMxcUri, OwnedRoomId, OwnedUserId};
pub use tokio::sync::mpsc;
pub use tokio::sync::oneshot;
