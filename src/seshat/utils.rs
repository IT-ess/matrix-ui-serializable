use matrix_sdk::ruma::{
    OwnedRoomId, events::room::message::SyncRoomMessageEvent, serde::JsonObject,
};
use seshat::{Error as SeshatError, RecoveryDatabase};

use anyhow::Result;

pub fn sync_to_seshat_event(
    sync_event: SyncRoomMessageEvent,
    room_id: OwnedRoomId,
) -> Option<seshat::Event> {
    let content = if let Some(original) = sync_event.as_original() {
        original.content.clone()
    } else {
        return None;
    };

    let mut source_json = JsonObject::new();
    source_json.insert(
        "body".to_string(),
        serde_json::to_value(content.body()).expect("couldn't serialize body value"),
    );
    source_json.insert(
        "event_id".to_string(),
        serde_json::to_value(sync_event.event_id()).expect("couldn't serialize event_id value"),
    );
    source_json.insert(
        "sender_id".to_string(),
        serde_json::to_value(sync_event.sender()).expect("couldn't serialize sender_id value"),
    );
    source_json.insert(
        "timestamp".to_string(),
        serde_json::to_value(sync_event.origin_server_ts().get())
            .expect("couldn't serialize timestamp value"),
    );
    source_json.insert(
        "room_id".to_string(),
        serde_json::to_value(room_id.to_string()).expect("couldn't serialize room_id value"),
    );
    source_json.insert(
        "msgtype".to_string(),
        serde_json::to_value(content.msgtype()).expect("couldn't serialize msgtype value"),
    );

    Some(seshat::Event {
        event_type: seshat::EventType::Message, // We only index messages for now
        content_value: content.body().to_string(),
        msgtype: Some(content.msgtype().to_string()),
        event_id: sync_event.event_id().to_string(),
        sender: sync_event.sender().to_string(),
        server_ts: sync_event.origin_server_ts().0.into(),
        room_id: room_id.to_string(),
        source: serde_json::to_string(&source_json)
            .expect("couldn't serialize source json to string"),
    })
}

// Helper function for the manual reindex logic (can be kept separate or inlined)
// This function now takes ownership of RecoveryDatabase and closes it.
pub fn perform_manual_reindex(mut recovery_db: RecoveryDatabase) -> Result<(), SeshatError> {
    println!("[Util] Starting manual reindex process using RecoveryDatabase...");

    // 1. Delete the existing index files
    println!("[Util] Deleting existing index...");
    recovery_db.delete_the_index()?;

    // 2. Re-open the index (now empty)
    println!("[Util] Opening new empty index...");
    recovery_db.open_index()?; // This prepares the internal index writer

    let batch_size = 500;
    println!("[Util] Loading and indexing source events in batches of {batch_size}...");

    // 3. Load the first batch of source events
    let mut current_batch = recovery_db.load_events_deserialized(batch_size, None)?;
    if !current_batch.is_empty() {
        println!(
            "[Util] Indexing first batch ({} events)...",
            current_batch.len()
        );
        recovery_db.index_events(&current_batch)?;
    } else {
        println!("[Util] No source events found to index.");
    }

    // 4. Loop through subsequent batches
    while !current_batch.is_empty() {
        let last_event_cursor = current_batch.last();
        current_batch = recovery_db.load_events_deserialized(batch_size, last_event_cursor)?;

        if current_batch.is_empty() {
            println!("[Util] No more events in subsequent batches.");
            break;
        }

        println!(
            "[Util] Indexing next batch ({} events)...",
            current_batch.len()
        );
        recovery_db.index_events(&current_batch)?;

        // Commit periodically
        println!("[Util] Committing batch...");
        recovery_db.commit()?;
    }

    // 5. Final commit and close
    println!("[Util] Final commit and close...");
    recovery_db.commit_and_close()?; // Consumes the recovery_db instance

    println!("[Util] Manual reindex process completed successfully.");
    Ok(())
}
