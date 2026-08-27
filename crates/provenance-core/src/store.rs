use core::fmt;
use provenance_domain::{EventEnvelope, EventSequence, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedVersion {
    Empty,
    Exact(EventSequence),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventStoreError {
    Conflict {
        expected: ExpectedVersion,
        actual: Option<EventSequence>,
    },
    InvalidSequence {
        expected: EventSequence,
        actual: EventSequence,
    },
    SequenceExhausted,
    Unavailable(String),
    Corrupt(String),
}

impl fmt::Display for EventStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { expected, actual } => write!(
                formatter,
                "event stream version conflict: expected {expected:?}, actual {actual:?}"
            ),
            Self::InvalidSequence { expected, actual } => write!(
                formatter,
                "event sequence is not contiguous: expected {expected}, actual {actual}"
            ),
            Self::SequenceExhausted => write!(formatter, "event sequence is exhausted"),
            Self::Unavailable(detail) => write!(formatter, "event store unavailable: {detail}"),
            Self::Corrupt(detail) => write!(formatter, "event store corrupt: {detail}"),
        }
    }
}

impl std::error::Error for EventStoreError {}

pub trait EventStore {
    fn append(
        &mut self,
        expected: ExpectedVersion,
        event: EventEnvelope,
    ) -> Result<(), EventStoreError>;

    fn load(&self, session_id: SessionId) -> Result<Vec<EventEnvelope>, EventStoreError>;
}
