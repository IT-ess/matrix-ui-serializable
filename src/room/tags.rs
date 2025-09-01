use matrix_sdk::ruma::events::tag::Tags;
use serde::{Serialize, Serializer};

// Newtype for FrontendRoomTags
#[derive(Debug, Clone)]
pub struct FrontendRoomTags(Tags);

impl FrontendRoomTags {
    #[allow(dead_code)]
    pub fn inner(&self) -> &Tags {
        &self.0
    }
}

impl From<Tags> for FrontendRoomTags {
    fn from(tags: Tags) -> Self {
        FrontendRoomTags(tags)
    }
}

impl Serialize for FrontendRoomTags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_newtype_struct("FrontendRoomTags", &self.0)
    }
}
