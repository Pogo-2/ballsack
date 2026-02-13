//! In-memory [`RoomState`] adapter (client-side peer tracking).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::application::ports::RoomState;
use crate::domain::identity::{PeerId, PeerInfo};

/// Simple in-memory room state.
pub struct InMemoryRoomState {
    peers: Mutex<HashMap<u64, PeerInfo>>,
}

impl InMemoryRoomState {
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryRoomState {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomState for InMemoryRoomState {
    fn peers(&self) -> Vec<PeerInfo> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    fn add_peer(&self, peer: PeerInfo) {
        self.peers.lock().unwrap().insert(peer.peer_id.0, peer);
    }

    fn remove_peer(&self, peer_id: PeerId) {
        self.peers.lock().unwrap().remove(&peer_id.0);
    }

    fn get_peer(&self, peer_id: PeerId) -> Option<PeerInfo> {
        self.peers.lock().unwrap().get(&peer_id.0).cloned()
    }
}
