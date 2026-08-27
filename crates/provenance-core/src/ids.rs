use provenance_domain::{EventId, SessionId};

pub trait IdGenerator {
    fn next_session_id(&mut self) -> SessionId;
    fn next_event_id(&mut self) -> EventId;
}
