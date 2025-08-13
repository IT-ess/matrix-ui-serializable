use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use matrix_sdk::Client;

use std::path::PathBuf;

use matrix_sdk::authentication::matrix::MatrixSession;

use crate::{
    events::events::{self, add_event_handlers},
    init::login,
    init::singletons::CLIENT,
    room::notifications::{MobilePushNotificationConfig, register_notifications},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientSession {
    homeserver: String,
    db_path: PathBuf,
    passphrase: String,
}

impl ClientSession {
    pub fn new(homeserver: String, db_path: PathBuf, passphrase: String) -> Self {
        ClientSession {
            homeserver,
            db_path,
            passphrase,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FullMatrixSession {
    client_session: ClientSession,
    user_session: MatrixSession,
}

impl FullMatrixSession {
    pub fn new(client_session: ClientSession, user_session: MatrixSession) -> Self {
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

    let client = Client::builder()
        .homeserver_url(client_session.homeserver)
        .sqlite_store(client_session.db_path, Some(&client_session.passphrase))
        .build()
        .await?;

    client.restore_session(user_session).await?;

    CLIENT
        .set(client.clone())
        .expect("BUG: CLIENT already set!");

    Ok(client)
}

pub async fn login_and_get_session(
    request: login::LoginRequest,
    mobile_push_config: Option<MobilePushNotificationConfig>,
    app_data_dir: PathBuf,
) -> crate::Result<String> {
    let (initial_client, session) =
        login::get_client_from_new_session(request, app_data_dir).await?;
    let client_with_handlers = events::add_event_handlers(initial_client)?;
    register_notifications(&client_with_handlers, mobile_push_config).await;
    Ok(session)
}

pub async fn try_restore_session_to_state(
    session_option: Option<String>,
    mobile_push_config: Option<MobilePushNotificationConfig>,
) -> crate::Result<Option<Client>> {
    match session_option {
        None => Ok(None),
        Some(session_string) => {
            let session: FullMatrixSession =
                serde_json::from_str(&session_string).map_err(|e| anyhow!(e))?;
            let initial_client = restore_client_from_session(session).await?;
            let client_with_handlers = add_event_handlers(initial_client)?;
            register_notifications(&client_with_handlers, mobile_push_config).await;
            Ok(Some(client_with_handlers))
        }
    }
}
