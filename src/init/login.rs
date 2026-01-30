use matrix_sdk::{
    Client, ThreadingSupport, config::RequestConfig, encryption::EncryptionSettings,
    sliding_sync::VersionBuilder,
};

use rand::{Rng, distr::Alphanumeric, rng};

use crate::{
    events::handlers::add_event_handlers,
    init::singletons::{APP_DATA_DIR, TEMP_CLIENT_SESSION},
};

use super::session::ClientSession;

pub(crate) async fn login_and_persist_matrix_session(
    client: &Client,
    username: String,
    password: String,
    device_name: String,
) -> anyhow::Result<String> {
    let matrix_auth = client.matrix_auth();

    matrix_auth
        .login_username(username, &password)
        .initial_device_display_name(&device_name)
        .await?;

    let user_session = matrix_auth
        .session()
        .expect("Should have session after login");

    let full = super::session::FullMatrixSession::new(
        TEMP_CLIENT_SESSION.wait().clone(),
        matrix_sdk::AuthSession::Matrix(user_session),
    );
    let serialized = serde_json::to_string(&full)?;

    Ok(serialized)
}

pub async fn build_client(
    homeserver_opt: Option<String>,
    client_session: Option<ClientSession>,
) -> anyhow::Result<(Client, ClientSession)> {
    let (homeserver, db_path, passphrase, db_identifier) = match client_session {
        Some(s) => {
            let db_path = APP_DATA_DIR
                .wait()
                .clone()
                .join("matrix-db")
                .join(&s.db_identifier);
            (s.homeserver, db_path, s.passphrase, s.db_identifier)
        }
        None => {
            if let Some(homeserver) = homeserver_opt {
                let db_identifier: String = rng()
                    .sample_iter(Alphanumeric)
                    .take(7)
                    .map(char::from)
                    .collect();
                let db_path = APP_DATA_DIR
                    .wait()
                    .clone()
                    .join("matrix-db")
                    .join(&db_identifier);

                std::fs::create_dir_all(&db_path)?;

                let passphrase: String = rng()
                    .sample_iter(Alphanumeric)
                    .take(32)
                    .map(char::from)
                    .collect();
                (homeserver, db_path, passphrase, db_identifier)
            } else {
                panic!("Cannot recover session from storage or create it without homeserver")
            }
        }
    };

    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver.clone())
        .with_threading_support(ThreadingSupport::Enabled {
            with_subscriptions: false,
        })
        .sqlite_store(&db_path, Some(&passphrase))
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: true,
            backup_download_strategy: matrix_sdk::encryption::BackupDownloadStrategy::OneShot,
            auto_enable_backups: true,
        })
        .with_enable_share_history_on_invite(true)
        .handle_refresh_tokens()
        .request_config(RequestConfig::new().timeout(std::time::Duration::from_secs(60)))
        .build()
        .await?;

    add_event_handlers(&client)?;

    let client_session = ClientSession::new(homeserver, db_identifier, passphrase);

    Ok((client, client_session))
}
