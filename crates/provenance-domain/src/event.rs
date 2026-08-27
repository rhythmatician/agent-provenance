use crate::{
    EventId, EventSequence, NativePath, NativeString, ObservationTime, ProcessInstanceId,
    SessionId, SourceId, UnixNanos, WorkspaceState, WorkspaceTransition,
};

pub const EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    executable: NativePath,
    arguments: Vec<NativeString>,
    working_directory: NativePath,
}

impl CommandSpec {
    pub fn new(
        executable: NativePath,
        arguments: Vec<NativeString>,
        working_directory: NativePath,
    ) -> Self {
        Self {
            executable,
            arguments,
            working_directory,
        }
    }

    pub fn executable(&self) -> &NativePath {
        &self.executable
    }

    pub fn arguments(&self) -> &[NativeString] {
        &self.arguments
    }

    pub fn working_directory(&self) -> &NativePath {
        &self.working_directory
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationSourceKind {
    Recorder,
    Process,
    FileSystem,
    Workspace,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationSource {
    id: SourceId,
    kind: ObservationSourceKind,
}

impl ObservationSource {
    pub const fn new(id: SourceId, kind: ObservationSourceKind) -> Self {
        Self { id, kind }
    }

    pub const fn recorder() -> Self {
        Self {
            id: SourceId::from_u128(0),
            kind: ObservationSourceKind::Recorder,
        }
    }

    pub const fn id(self) -> SourceId {
        self.id
    }

    pub const fn kind(self) -> ObservationSourceKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStarted {
    command: CommandSpec,
    initial_workspace: Option<WorkspaceState>,
}

impl SessionStarted {
    pub fn new(command: CommandSpec, initial_workspace: Option<WorkspaceState>) -> Self {
        Self {
            command,
            initial_workspace,
        }
    }

    pub fn command(&self) -> &CommandSpec {
        &self.command
    }

    pub fn initial_workspace(&self) -> Option<&WorkspaceState> {
        self.initial_workspace.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOutcome {
    Completed,
    Aborted,
    CaptureFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEnded {
    outcome: SessionOutcome,
    final_workspace: Option<WorkspaceState>,
}

impl SessionEnded {
    pub fn new(outcome: SessionOutcome, final_workspace: Option<WorkspaceState>) -> Self {
        Self {
            outcome,
            final_workspace,
        }
    }

    pub const fn outcome(&self) -> SessionOutcome {
        self.outcome
    }

    pub fn final_workspace(&self) -> Option<&WorkspaceState> {
        self.final_workspace.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessStarted {
    pub process_id: ProcessInstanceId,
    pub parent_process_id: Option<ProcessInstanceId>,
    pub operating_system_pid: Option<u32>,
    pub command: CommandSpec,
    pub workspace_state: Option<WorkspaceState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessTermination {
    ExitCode(i32),
    Signal(i32),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessExited {
    pub process_id: ProcessInstanceId,
    pub termination: ProcessTermination,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileMutationKind {
    Created,
    Modified,
    Renamed { from: NativePath },
    Deleted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMutationObserved {
    pub path: NativePath,
    pub kind: FileMutationKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceStateAdvanced {
    pub transition: WorkspaceTransition,
    pub cause_event: Option<EventId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GapScope {
    ProcessTree,
    FileSystem,
    WorkspaceState,
    Output,
    CaptureAdapter,
    Recorder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GapReason {
    Unsupported,
    BufferOverflow,
    PermissionDenied,
    ObserverFailed,
    RecorderRestarted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationGap {
    pub scope: GapScope,
    pub reason: GapReason,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeObservationKind {
    ProcessStarted(ProcessStarted),
    ProcessExited(ProcessExited),
    FileMutationObserved(FileMutationObserved),
    WorkspaceStateAdvanced(WorkspaceStateAdvanced),
    ObservationGap(ObservationGap),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeObservation {
    source: ObservationSource,
    observed_at: ObservationTime,
    kind: RuntimeObservationKind,
}

impl RuntimeObservation {
    pub fn new(
        source: ObservationSource,
        observed_at: ObservationTime,
        kind: RuntimeObservationKind,
    ) -> Self {
        Self {
            source,
            observed_at,
            kind,
        }
    }

    pub fn recorder_gap(scope: GapScope, reason: GapReason, detail: String) -> Self {
        Self::new(
            ObservationSource::recorder(),
            ObservationTime::unknown(),
            RuntimeObservationKind::ObservationGap(ObservationGap {
                scope,
                reason,
                detail,
            }),
        )
    }

    pub const fn source(&self) -> ObservationSource {
        self.source
    }

    pub const fn observed_at(&self) -> ObservationTime {
        self.observed_at
    }

    pub fn kind(&self) -> &RuntimeObservationKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Observation {
    SessionStarted(SessionStarted),
    Runtime(RuntimeObservation),
    SessionEnded(SessionEnded),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    schema_version: u16,
    event_id: EventId,
    session_id: SessionId,
    sequence: EventSequence,
    recorded_at: UnixNanos,
    observation: Observation,
}

impl EventEnvelope {
    pub fn new(
        event_id: EventId,
        session_id: SessionId,
        sequence: EventSequence,
        recorded_at: UnixNanos,
        observation: Observation,
    ) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            event_id,
            session_id,
            sequence,
            recorded_at,
            observation,
        }
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }

    pub const fn recorded_at(&self) -> UnixNanos {
        self.recorded_at
    }

    pub fn observation(&self) -> &Observation {
        &self.observation
    }
}
