use matrix_sdk::ruma::{
    OwnedMxcUri, OwnedUserId, api::client::user_directory::search_users::v3::User,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModel {
    pub user_id: OwnedUserId,
    pub display_name: Option<String>,
    pub avatar_url: Option<OwnedMxcUri>,
}

impl From<User> for ProfileModel {
    fn from(value: User) -> Self {
        Self {
            user_id: value.user_id,
            display_name: value.display_name,
            avatar_url: value.avatar_url,
        }
    }
}
