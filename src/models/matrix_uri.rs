use matrix_sdk::ruma::matrix_uri::MatrixId;
use matrix_sdk::ruma::{MatrixToUri, MatrixUri, OwnedEventId, OwnedRoomOrAliasId, OwnedUserId};
use matrix_sdk::{IdParseError, OwnedServerName};
use serde::Serialize;

use crate::UserProfile;
use crate::commands::SerializableRoomPreview;

/// Tries to extract a room address (Alias or ID) from the given text.
///
/// This function is quite flexible and will attempt to parse `text` as:
/// * A Room ID (with a leading `!`).
/// * A Room Alias (with a leading `#`).
/// * A `https://matrix.to` URI, which includes either a room alias, or a room ID plus `via` servers.
/// * A `matrix:` scheme URI, which is similar to above.
pub(crate) fn parse_address(
    text: &str,
) -> Result<(OwnedRoomOrAliasId, Vec<OwnedServerName>), IdParseError> {
    match OwnedRoomOrAliasId::try_from(text) {
        Ok(room_or_alias_id) => Ok((room_or_alias_id, Vec::new())),
        Err(e) => {
            let uri_result = MatrixToUri::parse(text)
                .map(|uri| (uri.id().clone(), uri.via().to_owned()))
                .or_else(|_| {
                    MatrixUri::parse(text).map(|uri| (uri.id().clone(), uri.via().to_owned()))
                });

            if let Ok((matrix_id, via)) = uri_result
                && let Some(room_or_alias_id) = match matrix_id {
                    MatrixId::Room(room_id) => Some(room_id.into()),
                    MatrixId::RoomAlias(alias) => Some(alias.into()),
                    MatrixId::Event(room_or_alias_id, _) => Some(room_or_alias_id),
                    _ => None,
                }
            {
                return Ok((room_or_alias_id, via));
            }
            Err(e)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "payload"
)]
pub enum MatrixUriIntent {
    Room(
        (
            OwnedRoomOrAliasId,
            Vec<OwnedServerName>,
            Option<OwnedEventId>,
        ),
    ),
    User(OwnedUserId),
}

pub fn get_matrix_uri_intent(text: &str) -> Result<MatrixUriIntent, IdParseError> {
    let (id, via) = MatrixToUri::parse(text)
        .map(|uri| (uri.id().clone(), uri.via().to_owned()))
        .or_else(|_| MatrixUri::parse(text).map(|uri| (uri.id().clone(), uri.via().to_owned())))?;

    match id {
        MatrixId::Room(room_id) => Ok(MatrixUriIntent::Room((room_id.into(), via, None))),
        MatrixId::RoomAlias(alias) => Ok(MatrixUriIntent::Room((alias.into(), via, None))),
        MatrixId::Event(room_or_alias_id, event_id) => Ok(MatrixUriIntent::Room((
            room_or_alias_id,
            via,
            Some(event_id),
        ))),
        MatrixId::User(user_id) => Ok(MatrixUriIntent::User(user_id)),
        _ => Err(IdParseError::InvalidMatrixUri(
            matrix_sdk::ruma::MatrixUriError::UnknownQueryItem,
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "payload"
)]
pub enum MatrixUriPillInfo {
    Room(
        (
            SerializableRoomPreview,
            Vec<OwnedServerName>,
            Option<OwnedEventId>,
        ),
    ),
    User(Option<UserProfile>),
}
