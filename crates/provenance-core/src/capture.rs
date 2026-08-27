use core::fmt;
use provenance_domain::{
    CommandSpec, GapReason, GapScope, RuntimeObservation, SessionOutcome, WorkspaceState,
};

use crate::{Clock, EventStore, IdGenerator, RecorderError, SessionRecorder};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRequest {
    command: CommandSpec,
    initial_workspace: Option<WorkspaceState>,
    workspace_scope: Option<provenance_domain::WorkspaceScope>,
}

impl CaptureRequest {
    pub fn new(command: CommandSpec, initial_workspace: Option<WorkspaceState>) -> Self {
        Self {
            command,
            initial_workspace,
            workspace_scope: None,
        }
    }

    pub fn with_scope(
        command: CommandSpec,
        initial_workspace: Option<WorkspaceState>,
        workspace_scope: provenance_domain::WorkspaceScope,
    ) -> Self {
        Self {
            command,
            initial_workspace,
            workspace_scope: Some(workspace_scope),
        }
    }

    pub fn command(&self) -> &CommandSpec {
        &self.command
    }

    pub fn initial_workspace(&self) -> Option<&WorkspaceState> {
        self.initial_workspace.as_ref()
    }

    pub fn workspace_scope(&self) -> Option<&provenance_domain::WorkspaceScope> {
        self.workspace_scope.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOutcome {
    session_outcome: SessionOutcome,
    final_workspace: Option<WorkspaceState>,
}

impl CaptureOutcome {
    pub fn new(session_outcome: SessionOutcome, final_workspace: Option<WorkspaceState>) -> Self {
        Self {
            session_outcome,
            final_workspace,
        }
    }

    pub const fn session_outcome(&self) -> SessionOutcome {
        self.session_outcome
    }

    pub fn final_workspace(&self) -> Option<&WorkspaceState> {
        self.final_workspace.as_ref()
    }

    fn into_parts(self) -> (SessionOutcome, Option<WorkspaceState>) {
        (self.session_outcome, self.final_workspace)
    }
}

pub trait ObservationSink {
    fn record(&mut self, observation: RuntimeObservation) -> Result<(), RecorderError>;
}

impl<S, C, I> ObservationSink for SessionRecorder<S, C, I>
where
    S: EventStore,
    C: Clock,
    I: IdGenerator,
{
    fn record(&mut self, observation: RuntimeObservation) -> Result<(), RecorderError> {
        self.record_observation(observation).map(|_| ())
    }
}

pub trait ExecutionCapture {
    fn capture(
        &mut self,
        request: &CaptureRequest,
        sink: &mut dyn ObservationSink,
    ) -> Result<CaptureOutcome, CaptureError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureError {
    Unsupported(String),
    Failed(String),
    ObservationSink(RecorderError),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(detail) => write!(formatter, "capture unsupported: {detail}"),
            Self::Failed(detail) => write!(formatter, "capture failed: {detail}"),
            Self::ObservationSink(source) => write!(formatter, "observation sink failed: {source}"),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<RecorderError> for CaptureError {
    fn from(source: RecorderError) -> Self {
        Self::ObservationSink(source)
    }
}

pub struct RecordedExecution<S, C, I> {
    session: crate::CompletedSession<S, C, I>,
    capture_error: Option<CaptureError>,
}

impl<S, C, I> RecordedExecution<S, C, I> {
    pub fn session(&self) -> &crate::CompletedSession<S, C, I> {
        &self.session
    }

    pub fn capture_error(&self) -> Option<&CaptureError> {
        self.capture_error.as_ref()
    }

    pub fn into_session(self) -> crate::CompletedSession<S, C, I> {
        self.session
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordExecutionError {
    Recorder(RecorderError),
}

impl fmt::Display for RecordExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recorder(source) => write!(formatter, "session recorder failed: {source}"),
        }
    }
}

impl std::error::Error for RecordExecutionError {}

impl From<RecorderError> for RecordExecutionError {
    fn from(source: RecorderError) -> Self {
        Self::Recorder(source)
    }
}

pub fn record_execution<S, C, I, X>(
    store: S,
    clock: C,
    ids: I,
    capture: &mut X,
    request: CaptureRequest,
) -> Result<RecordedExecution<S, C, I>, RecordExecutionError>
where
    S: EventStore,
    C: Clock,
    I: IdGenerator,
    X: ExecutionCapture,
{
    let mut recorder = SessionRecorder::start(
        store,
        clock,
        ids,
        request.command().clone(),
        request.initial_workspace().cloned(),
    )?;

    match capture.capture(&request, &mut recorder) {
        Ok(outcome) => {
            let (session_outcome, final_workspace) = outcome.into_parts();
            let session = recorder.finish(session_outcome, final_workspace)?;
            Ok(RecordedExecution {
                session,
                capture_error: None,
            })
        }
        Err(CaptureError::ObservationSink(source)) => Err(RecordExecutionError::Recorder(source)),
        Err(error) => {
            recorder.record_observation(RuntimeObservation::recorder_gap(
                GapScope::CaptureAdapter,
                GapReason::ObserverFailed,
                error.to_string(),
            ))?;
            let session = recorder.finish(SessionOutcome::CaptureFailed, None)?;
            Ok(RecordedExecution {
                session,
                capture_error: Some(error),
            })
        }
    }
}
