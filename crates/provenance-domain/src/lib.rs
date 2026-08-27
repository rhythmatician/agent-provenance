#![forbid(unsafe_code)]

pub mod event;
pub mod identity;
pub mod native;
pub mod state;
pub mod time;

pub use event::{
    CommandSpec, EVENT_SCHEMA_VERSION, EventEnvelope, FileMutationKind, FileMutationObserved,
    GapReason, GapScope, Observation, ObservationGap, ObservationSource, ObservationSourceKind,
    ProcessExited, ProcessStarted, ProcessTermination, RuntimeObservation, RuntimeObservationKind,
    SessionEnded, SessionOutcome, SessionStarted, WorkspaceStateAdvanced,
};
pub use identity::{EventId, EventSequence, ProcessInstanceId, SessionId, SourceId};
pub use native::{NativeEncoding, NativePath, NativeString, NativeStringError};
pub use state::{
    ContentDigest, DigestAlgorithm, StateError, WorkspaceGeneration, WorkspaceState,
    WorkspaceTransition,
};
pub use time::{MonotonicNanos, ObservationTime, UnixNanos};
