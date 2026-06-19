use std::ops::{Deref, DerefMut};

use matrix_sdk::ruma::{OwnedEventId, OwnedTransactionId};
use matrix_sdk_ui::timeline::TimelineEventItemId;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct FrontendTimelineEventItemId(pub(super) TimelineEventItemId);

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

        match value {
            Value::String(s) => {
                if let Ok(id) = OwnedEventId::try_from(s.clone()) {
                    Ok(FrontendTimelineEventItemId::from(
                        TimelineEventItemId::EventId(id),
                    ))
                } else {
                    Ok(FrontendTimelineEventItemId::from(
                        TimelineEventItemId::TransactionId(OwnedTransactionId::from(s)),
                    ))
                }
            }
            _value => Err(serde::de::Error::custom("Only support strings")),
        }
    }
}

impl From<TimelineEventItemId> for FrontendTimelineEventItemId {
    fn from(item_id: TimelineEventItemId) -> Self {
        FrontendTimelineEventItemId(item_id)
    }
}
