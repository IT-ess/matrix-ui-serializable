use matrix_sdk::{
    AuthSession,
    encryption::recovery::RecoveryState,
    ruma::{OwnedMxcUri, OwnedUserId},
};

use crate::{
    room::{room_screen::RoomScreen, rooms_list::RoomsList},
    stores::login_store::{FrontendSyncServiceState, FrontendVerificationState, LoginState},
    user::user_profile::UserProfileMap,
};
use async_trait::async_trait;

/// Super trait that defines the required "updaters" functions that will translate a library
/// state change into a frontend one.
pub trait StateUpdater: StateUpdaterFunctions + std::fmt::Debug + Send + Sync {}

#[async_trait]
pub trait StateUpdaterFunctions {
    fn update_rooms_list(&self, rooms_list: &RoomsList) -> anyhow::Result<()>;
    fn update_room(&self, room: &RoomScreen, room_id: &str) -> anyhow::Result<()>;
    fn update_sync_service(
        &self,
        sync_service_state: FrontendSyncServiceState,
    ) -> anyhow::Result<()>;
    fn update_profile(&self, user_profiles: &UserProfileMap) -> anyhow::Result<()>;
    fn update_login_state(
        &self,
        login_state: LoginState,
        user_id: Option<String>,
    ) -> anyhow::Result<()>;
    fn update_verification_state(
        &self,
        verification_state: FrontendVerificationState,
    ) -> anyhow::Result<()>;
    fn update_recovery_state(&self, recovery_state: RecoveryState) -> anyhow::Result<()>;
    fn update_current_user_info(
        &self,
        current_user_id: Option<OwnedUserId>,
        user_avatar: Option<OwnedMxcUri>,
        user_display_name: Option<String>,
        device_display_name: Option<String>,
    ) -> anyhow::Result<()>;
    async fn persist_refreshed_session(&self, refreshed_session: AuthSession)
    -> anyhow::Result<()>;
    async fn persist_login_session(&self, session: String) -> anyhow::Result<()>;
}
