#![forbid(unsafe_code)]

pub mod capture;
pub mod clock;
pub mod ids;
pub mod recorder;
pub mod store;
pub mod validation;

pub use capture::{
    CaptureError, CaptureOutcome, CaptureRequest, ExecutionCapture, ObservationSink,
    RecordExecutionError, RecordedExecution, record_execution,
};
pub use clock::Clock;
pub use ids::IdGenerator;
pub use recorder::{CompletedSession, RecordedEvent, RecorderError, SessionRecorder};
pub use store::{EventStore, EventStoreError, ExpectedVersion};
pub use validation::{EvidenceContinuity, EvidenceFreshness, assess_validation_freshness};
