use matrix_sdk::{Client, sliding_sync::VersionBuilder};

use std::path::PathBuf;

use rand::{Rng, distr::Alphanumeric, rng};
use serde::{Deserialize, Serialize};

use crate::init::singletons::CLIENT;

use super::session::ClientSession;

/// The user's account credentials to create a new Matrix session
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MatrixClientConfig {
    username: String,
    password: String,
    homeserver_url: String,
    client_name: String,
}

impl MatrixClientConfig {
    pub fn new(
        username: String,
        password: String,
        homeserver_url: String,
        client_name: String,
    ) -> Self {
        MatrixClientConfig {
            username,
            password,
            homeserver_url,
            client_name,
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }
}

/// Details of a login request that get submitted when calling login command
pub enum LoginRequest {
    LoginByPassword(MatrixClientConfig),
}

pub async fn get_client_from_new_session(
    login_request: LoginRequest,
    app_data_dir: PathBuf,
) -> anyhow::Result<(Client, String)> {
    let matrix_config = match login_request {
        LoginRequest::LoginByPassword(config) => config,
        // TODO: add new login ways
    };

    let client_initial_state = login_and_persist_session(&matrix_config, app_data_dir).await?;

    CLIENT
        .set(client_initial_state.0.clone())
        .expect("BUG: CLIENT already set!");

    Ok(client_initial_state)
}

async fn login_and_persist_session(
    config: &MatrixClientConfig,
    app_data_dir: PathBuf,
) -> anyhow::Result<(Client, String)> {
    let (client, client_session) = build_client(config, app_data_dir).await?;

    let matrix_auth = client.matrix_auth();

    matrix_auth
        .login_username(&config.username, &config.password)
        .initial_device_display_name(&config.client_name)
        .await?;

    let user_session = matrix_auth
        .session()
        .expect("Should have session after login");

    let full = super::session::FullMatrixSession::new(client_session, user_session);
    let serialized = serde_json::to_string(&full)?;

    Ok((client, serialized))
}

async fn build_client(
    config: &MatrixClientConfig,
    app_data_dir: PathBuf,
) -> anyhow::Result<(Client, ClientSession)> {
    let db_subfolder: String = rng()
        .sample_iter(Alphanumeric)
        .take(7)
        .map(char::from)
        .collect();
    let db_path = app_data_dir.join("matrix-db").join(db_subfolder);

    std::fs::create_dir_all(&db_path)?;

    let passphrase: String = rng()
        .sample_iter(Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let client = Client::builder()
        .homeserver_url(&config.homeserver_url)
        .sqlite_store(&db_path, Some(&passphrase))
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative) // Comment this if your homeserver doesn't support simplified sliding sync.
        .handle_refresh_tokens()
        .build()
        .await?;

    let client_session = ClientSession::new(config.homeserver_url.clone(), db_path, passphrase);

    Ok((client, client_session))
}
