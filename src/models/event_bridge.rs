use tokio::sync::broadcast;

use crate::models::requests::EmitEvent;

#[derive(Debug)]
pub struct EventBridge {
    sender: broadcast::Sender<EmitEvent>,
}

impl EventBridge {
    pub fn new() -> (Self, broadcast::Receiver<EmitEvent>) {
        let (sender, receiver) = broadcast::channel(100);
        (Self { sender }, receiver)
    }

    pub fn emit(&self, event: EmitEvent) {
        let _ = self.sender.send(event);
    }
}
