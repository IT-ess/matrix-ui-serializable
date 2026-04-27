use anyhow::anyhow;
use matrix_sdk::{Client, authentication::oauth::error::OAuthDiscoveryError};
use serde::Serialize;

use crate::init::singletons::TEMP_CLIENT;

pub(crate) mod login;
pub(crate) mod oauth;
pub(crate) mod session;
pub mod singletons;
mod sync;
pub(crate) mod workers;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FrontendAuthTypeResponse {
    Matrix,
    Oauth,
    WrongUrl,
}

pub async fn check_homeserver_auth_type() -> anyhow::Result<(FrontendAuthTypeResponse, Client)> {
    let client = {
        let mut guard = TEMP_CLIENT.lock.lock().unwrap();
        while guard.is_none() {
            guard = TEMP_CLIENT.cvar.wait(guard).unwrap();
        }
        guard.clone().ok_or(anyhow!("No temp client set yet"))?
    };
    match client.oauth().server_metadata().await {
        Ok(_) => Ok((FrontendAuthTypeResponse::Oauth, client)),
        Err(e) => match e {
            OAuthDiscoveryError::NotSupported => Ok((FrontendAuthTypeResponse::Matrix, client)),
            OAuthDiscoveryError::Url(_) => Ok((FrontendAuthTypeResponse::WrongUrl, client)),
            _ => Err(anyhow!("Unknown error when checking available auth types")),
        },
    }
}
