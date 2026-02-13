//! CBOR-based ControlCodec implementation.

use crate::application::ports::ControlCodec;
use crate::domain::control::ControlMsg;

/// Encodes / decodes [`ControlMsg`] using CBOR (via `serde_cbor`).
pub struct CborControlCodec;

impl ControlCodec for CborControlCodec {
    fn encode(&self, msg: &ControlMsg) -> anyhow::Result<Vec<u8>> {
        serde_cbor::to_vec(msg).map_err(Into::into)
    }

    fn decode(&self, data: &[u8]) -> anyhow::Result<ControlMsg> {
        serde_cbor::from_slice(data).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let codec = CborControlCodec;
        let msg = ControlMsg::Hello {
            client_version: "0.1.0".into(),
            device_info: "test".into(),
        };
        let bytes = codec.encode(&msg).unwrap();
        let decoded = codec.decode(&bytes).unwrap();
        // Verify variant matches (Debug comparison)
        assert!(matches!(decoded, ControlMsg::Hello { .. }));
    }
}
