#![forbid(unsafe_code)]

use getrandom::getrandom;
use provenance_core::IdGenerator;
use provenance_domain::{EventId, SessionId};

/// Random [`IdGenerator`] for production use.
///
/// Uses `getrandom` to generate cryptographically random `u128` values for
/// `SessionId` and `EventId`. Panics only if the platform RNG is unavailable,
/// which is treated as a hard failure for recording.
#[derive(Clone, Copy, Debug, Default)]
pub struct RandomIdGenerator;

impl IdGenerator for RandomIdGenerator {
    fn next_session_id(&mut self) -> SessionId {
        let mut bytes = [0u8; 16];
        getrandom(&mut bytes).expect("getrandom for session id");
        SessionId::from_u128(u128::from_le_bytes(bytes))
    }

    fn next_event_id(&mut self) -> EventId {
        let mut bytes = [0u8; 16];
        getrandom(&mut bytes).expect("getrandom for event id");
        EventId::from_u128(u128::from_le_bytes(bytes))
    }
}
