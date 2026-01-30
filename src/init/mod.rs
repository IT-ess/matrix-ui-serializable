use crate::init::singletons::CLIENT;
use anyhow::anyhow;
use matrix_sdk::authentication::oauth::error::OAuthDiscoveryError;
use serde::Serialize;
use ts_rs::TS;

pub(crate) mod login;
pub(crate) mod oauth;
pub(crate) mod session;
pub mod singletons;
mod sync;
pub(crate) mod workers;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum FrontendAuthTypeResponse {
    Matrix,
    Oauth,
    WrongUrl,
}

pub async fn check_homeserver_auth_type() -> anyhow::Result<FrontendAuthTypeResponse> {
    let client = CLIENT.wait();
    match client.oauth().server_metadata().await {
        Ok(_) => Ok(FrontendAuthTypeResponse::Oauth),
        Err(e) => match e {
            OAuthDiscoveryError::NotSupported => Ok(FrontendAuthTypeResponse::Matrix),
            OAuthDiscoveryError::Url(_) => Ok(FrontendAuthTypeResponse::WrongUrl),
            _ => Err(anyhow!("Unknown error when checking available auth types")),
        },
    }
}
