use matrix_sdk::{
    Client,
    authentication::oauth::{
        OAuthAuthorizationData, UrlOrQuery,
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
    },
    ruma::serde::Raw,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use url::Url;

use crate::{
    init::singletons::{TEMP_CLIENT_SESSION, get_event_bridge},
    models::events::EmitEvent,
};

/// Generate the OAuth 2.0 client metadata.
fn client_metadata(client_uri: Url, callback_url: Url) -> Raw<ClientMetadata> {
    let client_uri = Localized::new(client_uri, None);

    debug!("Using client URI {client_uri:?} and callback_url {callback_url:?}");

    let metadata = ClientMetadata {
        // The following fields should be displayed in the OAuth 2.0 authorization server's
        // web UI as part of the process to get the user's consent.
        client_name: Some(Localized::new("Matrix Svelte Client".to_owned(), [])),
        policy_uri: None,
        logo_uri: None,
        tos_uri: None,
        ..ClientMetadata::new(
            // This is a native application (in contrast to a web application, that runs in a
            // browser).
            ApplicationType::Native,
            // We are going to use the Authorization Code flow.
            vec![OAuthGrantType::AuthorizationCode {
                redirect_uris: vec![callback_url],
            }],
            client_uri,
        )
    };

    Raw::new(&metadata).expect("Couldn't serialize client metadata")
}

/// Register the client and log in the user via the OAuth 2.0 Authorization
/// Code flow.
pub(crate) async fn register_and_login_oauth(
    client: &Client,
    mut oauth_deeplink_receiver: mpsc::Receiver<Url>,
    client_uri: &Url,
    callback_url: &Url,
) -> anyhow::Result<String> {
    let oauth = client.oauth();

    // We create a loop here so the user can retry if an error happens.
    loop {
        let OAuthAuthorizationData { url, .. } = oauth
            .login(
                callback_url.to_owned(),
                None,
                Some(client_metadata(client_uri.to_owned(), callback_url.to_owned()).into()),
                None,
            )
            .build()
            .await?;

        // Send auth URL to frontend
        get_event_bridge()
            .expect("bridge should be defined at this point")
            .emit(EmitEvent::OAuthUrl(url.to_string()));

        let query_string = oauth_deeplink_receiver
            .recv()
            .await
            .expect("no url was sent");

        match oauth
            .finish_login(UrlOrQuery::Query(
                query_string
                    .query()
                    .expect("no query params passed to auth callback")
                    .to_owned(),
            ))
            .await
        {
            Ok(()) => {
                info!("Logged in");
                break;
            }
            Err(err) => {
                error!("Error: failed to login: {err}");
                continue;
            }
        }
    }

    let user_session = oauth
        .full_session()
        .expect("Should have session after login");

    let full = super::session::FullMatrixSession::new(
        TEMP_CLIENT_SESSION.wait().clone(),
        matrix_sdk::AuthSession::OAuth(Box::new(user_session)),
    );
    let serialized = serde_json::to_string(&full)?;

    Ok(serialized)
}
