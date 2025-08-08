use crate::{
    init::singletons::{get_event_bridge, get_room_created_receiver_lock},
    models::events::{EmitEvent, MatrixRoomStoreCreateRequest},
};

pub async fn send_room_creation_request_and_await_response(id: &str) -> anyhow::Result<()> {
    let event_bridge = get_event_bridge()?;

    event_bridge.emit(EmitEvent::RoomCreate(MatrixRoomStoreCreateRequest::new(
        id.to_string(),
    )));

    let mut receiver = get_room_created_receiver_lock().await?;

    while let Some(msg) = receiver.recv().await {
        if msg.id == id {
            break;
        }
    }

    Ok(())
}
