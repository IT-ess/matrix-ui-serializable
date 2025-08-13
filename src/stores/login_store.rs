use std::ops::{Deref, DerefMut};

use matrix_sdk::encryption::VerificationState;
use matrix_sdk_ui::sync_service;
use serde::{Serialize, Serializer, ser::SerializeStruct};

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LoginState {
    Initiating,
    Restored,
    AwaitingForLogin,
    LoggedIn,
}

impl LoginState {
    pub fn to_camel_case(&self) -> String {
        match self {
            LoginState::Initiating => "initiating".to_string(),
            LoginState::Restored => "restored".to_string(),
            LoginState::AwaitingForLogin => "awaitingForLogin".to_string(),
            LoginState::LoggedIn => "loggedIn".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FrontendVerificationState(VerificationState);

impl Deref for FrontendVerificationState {
    type Target = VerificationState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendVerificationState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FrontendVerificationState {
    pub fn new(state: VerificationState) -> Self {
        Self(state)
    }

    pub fn to_camel_case(&self) -> &str {
        match self {
            FrontendVerificationState(VerificationState::Unknown) => "unknown",
            FrontendVerificationState(VerificationState::Verified) => "verified",
            FrontendVerificationState(VerificationState::Unverified) => "unverified",
        }
    }
}

impl Serialize for FrontendVerificationState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FrontendVerificationState", 1)?;

        state.serialize_field("verificationState", &self.to_camel_case())?;

        state.end()
    }
}

pub struct FrontendSyncServiceState(sync_service::State);

impl Deref for FrontendSyncServiceState {
    type Target = sync_service::State;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendSyncServiceState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FrontendSyncServiceState {
    pub fn new(state: sync_service::State) -> Self {
        Self(state)
    }

    pub fn to_camel_case(&self) -> &str {
        match self {
            FrontendSyncServiceState(sync_service::State::Error) => "error",
            FrontendSyncServiceState(sync_service::State::Idle) => "idle",
            FrontendSyncServiceState(sync_service::State::Offline) => "offline",
            FrontendSyncServiceState(sync_service::State::Running) => "running",
            FrontendSyncServiceState(sync_service::State::Terminated) => "terminated",
        }
    }
}

impl Serialize for FrontendSyncServiceState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FrontendSyncServiceState", 1)?;

        state.serialize_field("syncServiceState", &self.to_camel_case())?;

        state.end()
    }
}
