use std::collections::BTreeMap;

use provenance_core::{EventStore, EventStoreError, ExpectedVersion};
use provenance_domain::{EventEnvelope, EventSequence, SessionId};

#[derive(Clone, Debug, Default)]
pub struct InMemoryEventStore {
    streams: BTreeMap<SessionId, Vec<EventEnvelope>>,
}

impl InMemoryEventStore {
    pub fn events(&self, session_id: SessionId) -> &[EventEnvelope] {
        self.streams.get(&session_id).map_or(&[], Vec::as_slice)
    }
}

impl EventStore for InMemoryEventStore {
    fn append(
        &mut self,
        expected: ExpectedVersion,
        event: EventEnvelope,
    ) -> Result<(), EventStoreError> {
        let stream = self.streams.entry(event.session_id()).or_default();
        let actual = stream.last().map(EventEnvelope::sequence);

        let version_matches = match expected {
            ExpectedVersion::Empty => actual.is_none(),
            ExpectedVersion::Exact(expected) => actual == Some(expected),
        };
        if !version_matches {
            return Err(EventStoreError::Conflict { expected, actual });
        }

        let expected_sequence = match actual {
            Some(sequence) => sequence
                .checked_next()
                .ok_or(EventStoreError::SequenceExhausted)?,
            None => EventSequence::ZERO,
        };
        if event.sequence() != expected_sequence {
            return Err(EventStoreError::InvalidSequence {
                expected: expected_sequence,
                actual: event.sequence(),
            });
        }

        stream.push(event);
        Ok(())
    }

    fn load(&self, session_id: SessionId) -> Result<Vec<EventEnvelope>, EventStoreError> {
        Ok(self.events(session_id).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use provenance_core::{EventStore, EventStoreError, ExpectedVersion};
    use provenance_domain::{
        CommandSpec, EventEnvelope, EventId, EventSequence, NativePath, Observation, SessionId,
        SessionStarted, UnixNanos,
    };

    use super::InMemoryEventStore;

    fn started_event(sequence: EventSequence) -> EventEnvelope {
        EventEnvelope::new(
            EventId::from_u128(2),
            SessionId::from_u128(1),
            sequence,
            UnixNanos::new(10),
            Observation::SessionStarted(SessionStarted::new(
                CommandSpec::new(
                    NativePath::from_unix_bytes(b"echo".to_vec()),
                    Vec::new(),
                    NativePath::from_unix_bytes(b"/tmp".to_vec()),
                ),
                None,
            )),
        )
    }

    #[test]
    fn wrong_expected_version_is_rejected() {
        let mut store = InMemoryEventStore::default();
        store
            .append(ExpectedVersion::Empty, started_event(EventSequence::ZERO))
            .expect("first event appends");

        let result = store.append(ExpectedVersion::Empty, started_event(EventSequence::new(1)));

        assert_eq!(
            Err(EventStoreError::Conflict {
                expected: ExpectedVersion::Empty,
                actual: Some(EventSequence::ZERO),
            }),
            result
        );
    }
}
