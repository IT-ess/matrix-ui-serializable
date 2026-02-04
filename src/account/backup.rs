use futures::StreamExt;
use matrix_sdk::encryption::recovery::EnableProgress;
use tracing::{info, warn};

use crate::init::singletons::CLIENT;

/// Checks whether this account has secret backup setup
pub async fn has_backup_setup() -> crate::Result<bool> {
    let client = CLIENT.wait();
    client
        .encryption()
        .backups()
        .fetch_exists_on_server()
        .await
        .map_err(|e| e.into())
}

/// Try to restore encryption keys from backup
pub async fn restore_backup_with_passphrase(passphrase: String) -> crate::Result<()> {
    info!("Restoring backup with passphrase");

    let client = CLIENT.wait();
    client
        .encryption()
        .recovery()
        .recover(&passphrase)
        .await
        .map_err(anyhow::Error::from)?;

    let status = client
        .encryption()
        .cross_signing_status()
        .await
        .expect("We should be able to get status");

    if status.is_complete() {
        info!("Successfully imported all the cross-signing keys");
    } else {
        warn!("Couldn't import all the cross-signing keys: {status:?}");
    }

    Ok(())
}

/// Setup a new backup for secret keys
pub async fn setup_new_backup() -> crate::Result<String> {
    let client = CLIENT.wait();
    let recovery = client.encryption().recovery();
    let enable = recovery.enable().wait_for_backups_to_upload();
    let mut progress_stream = enable.subscribe_to_progress();

    tokio::spawn(async move {
        while let Some(update) = progress_stream.next().await {
            let Ok(update) = update else {
                panic!("Update to the enable progress lagged")
            };

            match update {
                EnableProgress::CreatingBackup => {
                    info!("Creating a new backup");
                }
                EnableProgress::CreatingRecoveryKey => {
                    info!("Creating a new recovery key");
                }
                EnableProgress::Done { .. } => {
                    info!("Recovery has been enabled");
                    break;
                }
                _ => (),
            }
        }
    });

    let recovery_key = enable.await.map_err(anyhow::Error::from)?;

    Ok(recovery_key)
}
