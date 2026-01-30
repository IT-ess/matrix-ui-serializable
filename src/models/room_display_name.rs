use std::ops::{Deref, DerefMut};

use matrix_sdk::RoomDisplayName;
use serde::{Serialize, Serializer};
use ts_rs::TS;

#[derive(Debug, Clone, TS)]
pub struct FrontendRoomDisplayName(RoomDisplayName);

impl Deref for FrontendRoomDisplayName {
    type Target = RoomDisplayName;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendRoomDisplayName {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<RoomDisplayName> for FrontendRoomDisplayName {
    fn from(content: RoomDisplayName) -> Self {
        FrontendRoomDisplayName(content)
    }
}

impl Serialize for FrontendRoomDisplayName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("RoomDisplayName", 2)?;

        match &self.0 {
            RoomDisplayName::Named(name) => {
                state.serialize_field("kind", "named")?;
                state.serialize_field("name", name)?;
            }
            RoomDisplayName::Aliased(name) => {
                state.serialize_field("kind", "aliased")?;
                state.serialize_field("name", name)?;
            }
            RoomDisplayName::Calculated(name) => {
                state.serialize_field("kind", "calculated")?;
                state.serialize_field("name", name)?;
            }
            RoomDisplayName::EmptyWas(name) => {
                state.serialize_field("kind", "empty_was")?;
                state.serialize_field("name", name)?;
            }
            RoomDisplayName::Empty => {
                state.serialize_field("kind", "empty")?;
                // Don't serialize the name field for Empty variant
            }
        }

        state.end()
    }
}
