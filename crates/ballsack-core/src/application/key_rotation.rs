//! KeyRotation use case — periodically or on membership events, rotates sender keys.

use std::sync::Arc;
use std::time::Duration;

use tracing::info;

use crate::domain::identity::{RoomId, SenderKeyId, SenderSecret};

use super::key_distribute::KeyDistributeUseCase;

/// Wraps [`KeyDistributeUseCase`] with a periodic timer for key rotation.
pub struct KeyRotationUseCase {
    key_distribute: Arc<KeyDistributeUseCase>,
    room_id: RoomId,
    interval: Duration,
}

impl KeyRotationUseCase {
    pub fn new(
        key_distribute: Arc<KeyDistributeUseCase>,
        room_id: RoomId,
        interval: Duration,
    ) -> Self {
        Self {
            key_distribute,
            room_id,
            interval,
        }
    }

    /// Run a periodic rotation loop. Returns the latest key on each rotation
    /// via the provided callback.
    pub async fn run(
        &self,
        mut on_new_key: impl FnMut(SenderKeyId, SenderSecret),
    ) -> anyhow::Result<()> {
        loop {
            tokio::time::sleep(self.interval).await;
            info!("Key rotation triggered");
            let (kid, secret) = self.key_distribute.execute(self.room_id).await?;
            on_new_key(kid, secret);
        }
    }

    /// Trigger a one-shot rotation (e.g. on peer join/leave).
    pub async fn rotate_now(&self) -> anyhow::Result<(SenderKeyId, SenderSecret)> {
        info!("On-demand key rotation");
        self.key_distribute.execute(self.room_id).await
    }
}
