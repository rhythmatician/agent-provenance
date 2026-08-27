use core::fmt;
use provenance_domain::{
    CommandSpec, EventEnvelope, EventId, EventSequence, Observation, RuntimeObservation,
    SessionEnded, SessionId, SessionOutcome, SessionStarted, WorkspaceState,
};

use crate::{Clock, EventStore, EventStoreError, ExpectedVersion, IdGenerator};

pub struct SessionRecorder<S, C, I> {
    session_id: SessionId,
    last_sequence: EventSequence,
    store: S,
    clock: C,
    ids: I,
}

impl<S, C, I> SessionRecorder<S, C, I>
where
    S: EventStore,
    C: Clock,
    I: IdGenerator,
{
    pub fn start(
        mut store: S,
        mut clock: C,
        mut ids: I,
        command: CommandSpec,
        initial_workspace: Option<WorkspaceState>,
    ) -> Result<Self, RecorderError> {
        let session_id = ids.next_session_id();
        let event = EventEnvelope::new(
            ids.next_event_id(),
            session_id,
            EventSequence::ZERO,
            clock.now(),
            Observation::SessionStarted(SessionStarted::new(command, initial_workspace)),
        );
        store.append(ExpectedVersion::Empty, event)?;

        Ok(Self {
            session_id,
            last_sequence: EventSequence::ZERO,
            store,
            clock,
            ids,
        })
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn last_sequence(&self) -> EventSequence {
        self.last_sequence
    }

    pub fn record_observation(
        &mut self,
        observation: RuntimeObservation,
    ) -> Result<RecordedEvent, RecorderError> {
        let sequence = self
            .last_sequence
            .checked_next()
            .ok_or(RecorderError::SequenceExhausted)?;
        let event_id = self.ids.next_event_id();
        let event = EventEnvelope::new(
            event_id,
            self.session_id,
            sequence,
            self.clock.now(),
            Observation::Runtime(observation),
        );
        self.store
            .append(ExpectedVersion::Exact(self.last_sequence), event)?;
        self.last_sequence = sequence;

        Ok(RecordedEvent { event_id, sequence })
    }

    pub fn finish(
        mut self,
        outcome: SessionOutcome,
        final_workspace: Option<WorkspaceState>,
    ) -> Result<CompletedSession<S, C, I>, RecorderError> {
        let sequence = self
            .last_sequence
            .checked_next()
            .ok_or(RecorderError::SequenceExhausted)?;
        let event = EventEnvelope::new(
            self.ids.next_event_id(),
            self.session_id,
            sequence,
            self.clock.now(),
            Observation::SessionEnded(SessionEnded::new(outcome, final_workspace)),
        );
        self.store
            .append(ExpectedVersion::Exact(self.last_sequence), event)?;

        Ok(CompletedSession {
            session_id: self.session_id,
            final_sequence: sequence,
            store: self.store,
            clock: self.clock,
            ids: self.ids,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordedEvent {
    event_id: EventId,
    sequence: EventSequence,
}

impl RecordedEvent {
    pub const fn event_id(self) -> EventId {
        self.event_id
    }

    pub const fn sequence(self) -> EventSequence {
        self.sequence
    }
}

pub struct CompletedSession<S, C, I> {
    session_id: SessionId,
    final_sequence: EventSequence,
    store: S,
    clock: C,
    ids: I,
}

impl<S, C, I> CompletedSession<S, C, I> {
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn final_sequence(&self) -> EventSequence {
        self.final_sequence
    }

    pub fn into_parts(self) -> (S, C, I) {
        (self.store, self.clock, self.ids)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecorderError {
    Store(EventStoreError),
    SequenceExhausted,
}

impl fmt::Display for RecorderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(source) => write!(formatter, "{source}"),
            Self::SequenceExhausted => write!(formatter, "event sequence is exhausted"),
        }
    }
}

impl std::error::Error for RecorderError {}

impl From<EventStoreError> for RecorderError {
    fn from(source: EventStoreError) -> Self {
        Self::Store(source)
    }
}
