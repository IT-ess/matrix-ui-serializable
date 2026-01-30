use matrix_sdk::ruma::{OwnedEventId, OwnedUserId};
use matrix_sdk_ui::timeline::ThreadSummary;
use serde::Serialize;
use ts_rs::TS;

use crate::events::{
    event_preview::text_preview_of_timeline_item, handlers::get_sender_username_from_profile,
};

pub fn get_frontend_thread_summary(thread_summary: ThreadSummary) -> Option<FrontendThreadSummary> {
    match thread_summary.latest_event {
        matrix_sdk_ui::timeline::TimelineDetails::Ready(e) => {
            let sender_username =
                get_sender_username_from_profile(e.sender_profile).unwrap_or("".to_owned());
            let preview = text_preview_of_timeline_item(&e.content, &sender_username)
                .format_with(&sender_username, true);
            Some(FrontendThreadSummary {
                event_formatted_summary: preview,
                sender_id: e.sender,
                num_replies: thread_summary.num_replies,
                private_read_receipt_event_id: thread_summary.private_read_receipt_event_id,
                public_read_receipt_event_id: thread_summary.public_read_receipt_event_id,
            })
        }
        // TODO: handle other states
        _ => None,
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
/// The mapped thread summary containing the latest item in this thread + additionnal infos.
pub struct FrontendThreadSummary {
    event_formatted_summary: String,
    sender_id: OwnedUserId,
    num_replies: u32,
    private_read_receipt_event_id: Option<OwnedEventId>,
    public_read_receipt_event_id: Option<OwnedEventId>,
}
