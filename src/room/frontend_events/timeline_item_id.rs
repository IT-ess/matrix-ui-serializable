use std::ops::{Deref, DerefMut};

use matrix_sdk::ruma::{OwnedEventId, OwnedTransactionId};
use matrix_sdk_ui::timeline::TimelineEventItemId;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, TS)]
pub struct FrontendTimelineEventItemId(TimelineEventItemId);

impl FrontendTimelineEventItemId {
    pub fn inner(self) -> TimelineEventItemId {
        self.0
    }
}

impl Deref for FrontendTimelineEventItemId {
    type Target = TimelineEventItemId;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FrontendTimelineEventItemId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Serialize for FrontendTimelineEventItemId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FrontendTimelineEventItemId", 1)?;

        match self.0.clone() {
            TimelineEventItemId::EventId(id) => {
                state.serialize_field("timelineItemId", &id)?;
            }
            TimelineEventItemId::TransactionId(id) => {
                state.serialize_field("timelineItemId", &id)?;
            }
        }

        state.end()
    }
}

impl<'de> Deserialize<'de> for FrontendTimelineEventItemId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // First deserialize into a generic Value to inspect the structure
        let value = Value::deserialize(deserializer)?;

        // Extract the "id" field first
        let id = value
            .get("timelineItemId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("timelineItemId"))?;

        // Extract the "payload" field containing the variant data
        let is_local = value
            .get("isLocal")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| serde::de::Error::missing_field("isLocal"))?;

        if is_local {
            let owned_id = OwnedTransactionId::from(id);
            Ok(FrontendTimelineEventItemId::from(
                TimelineEventItemId::TransactionId(owned_id),
            ))
        } else {
            let owned_id = OwnedEventId::try_from(id).map_err(serde::de::Error::custom)?;
            Ok(FrontendTimelineEventItemId::from(
                TimelineEventItemId::EventId(owned_id),
            ))
        }
    }
}

impl From<TimelineEventItemId> for FrontendTimelineEventItemId {
    fn from(item_id: TimelineEventItemId) -> Self {
        FrontendTimelineEventItemId(item_id)
    }
}
