use crate::{
    room::{room_screen::RoomScreen, rooms_list::RoomsList},
    stores::login_store::{FrontendSyncServiceState, FrontendVerificationState, LoginState},
    user::user_profile::UserProfileMap,
};

pub trait StateUpdater: StateUpdaterFunctions + std::fmt::Debug + Send + Sync {}

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
}
