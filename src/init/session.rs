use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use matrix_sdk::{AuthSession, Client};

use std::sync::Arc;

use crate::{
    init::{login::build_client, singletons::CLIENT},
    models::{
        events::{ToastNotificationRequest, ToastNotificationVariant},
        state_updater::StateUpdater,
    },
    room::notifications::enqueue_toast_notification,
};

/// The data needed to re-build a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    /// The URL of the homeserver of the user.
    pub(crate) homeserver: String,

    /// The random identifier of the DB (to avoid collision with old data).
    /// We do not store the full path since it can change when updating on some devices (iOS for instance)
    pub(crate) db_identifier: String,

    /// The passphrase of the database.
    pub(crate) passphrase: String,
}

impl ClientSession {
    pub fn new(homeserver: String, db_identifier: String, passphrase: String) -> Self {
        ClientSession {
            homeserver,
            db_identifier,
            passphrase,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FullMatrixSession {
    pub client_session: ClientSession,
    pub user_session: AuthSession,
}

impl FullMatrixSession {
    pub fn new(client_session: ClientSession, user_session: AuthSession) -> Self {
        FullMatrixSession {
            client_session,
            user_session,
        }
    }
}

pub async fn restore_client_from_session(session: FullMatrixSession) -> anyhow::Result<Client> {
    let FullMatrixSession {
        client_session,
        user_session,
    } = session;

    let (client, _) = build_client(None, Some(client_session)).await?;

    client.restore_session(user_session).await?;

    CLIENT
        .set(client.clone())
        .expect("BUG: CLIENT already set!");

    Ok(client)
}

pub async fn try_restore_session_to_state(
    session_option: Option<String>,
) -> crate::Result<Option<Client>> {
    match session_option {
        None => Ok(None),
        Some(session_string) => {
            let session: FullMatrixSession =
                serde_json::from_str(&session_string).map_err(|e| anyhow!(e))?;
            let initial_client = restore_client_from_session(session).await?;
            Ok(Some(initial_client))
        }
    }
}

/// Sets up this client so that it automatically saves the session into keychain
/// whenever there are new tokens that have been received.
///
/// This should always be set up whenever automatic refresh is happening.
pub(crate) fn setup_token_background_save(updater: Arc<Box<dyn StateUpdater>>) {
    tokio::spawn(async move {
        let client = CLIENT.wait();
        while let Ok(update) = client.subscribe_to_session_changes().recv().await {
            match update {
                matrix_sdk::SessionChange::UnknownToken { soft_logout } => {
                    enqueue_toast_notification(ToastNotificationRequest::new(
                        format!("This session is no longer valid. Error: {soft_logout}"),
                        None,
                        ToastNotificationVariant::Error,
                    ));
                    warn!("Received an unknown token error; soft logout? {soft_logout:?}");
                }
                matrix_sdk::SessionChange::TokensRefreshed => {
                    // The tokens have been refreshed, persist them to disk.
                    if let Err(err) = update_stored_session(client, updater.clone()).await {
                        enqueue_toast_notification(ToastNotificationRequest::new(
                            format!("Failed to persist refreshed session. Error: {err}"),
                            None,
                            ToastNotificationVariant::Error,
                        ));
                        error!("Unable to store a session in the background: {err}");
                    }
                }
            }
        }
    });
}

/// Update the session stored in the keychain.
///
/// This should be called everytime the access token (and possibly refresh
/// token) has changed.
async fn update_stored_session(
    client: &Client,
    updater: Arc<Box<dyn StateUpdater>>,
) -> anyhow::Result<()> {
    info!("Updating the stored session...");

    let user_session = client
        .session()
        .ok_or(anyhow!("No auth session available to persist!"))?;

    updater
        .as_ref()
        .persist_refreshed_session(user_session)
        .await?;

    info!("Updating the stored session: done!");
    Ok(())
}
