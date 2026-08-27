use std::collections::VecDeque;

use provenance_adapters::InMemoryEventStore;
use provenance_core::{
    CaptureError, CaptureOutcome, CaptureRequest, Clock, ExecutionCapture, IdGenerator,
    ObservationSink, record_execution,
};
use provenance_domain::{
    CommandSpec, EventId, EventSequence, GapReason, GapScope, NativePath, NativeString,
    Observation, ObservationSource, ObservationSourceKind, ObservationTime, ProcessExited,
    ProcessInstanceId, ProcessStarted, ProcessTermination, RuntimeObservation,
    RuntimeObservationKind, SessionId, SessionOutcome, SourceId, UnixNanos, WorkspaceState,
};

struct FixedClock {
    values: VecDeque<UnixNanos>,
}

impl FixedClock {
    fn new(values: impl IntoIterator<Item = i64>) -> Self {
        Self {
            values: values.into_iter().map(UnixNanos::new).collect(),
        }
    }
}

impl Clock for FixedClock {
    fn now(&mut self) -> UnixNanos {
        self.values.pop_front().expect("fixed clock has a value")
    }
}

struct FixedIds {
    session_ids: VecDeque<SessionId>,
    event_ids: VecDeque<EventId>,
}

impl FixedIds {
    fn new(session_ids: &[u128], event_ids: &[u128]) -> Self {
        Self {
            session_ids: session_ids
                .iter()
                .copied()
                .map(SessionId::from_u128)
                .collect(),
            event_ids: event_ids.iter().copied().map(EventId::from_u128).collect(),
        }
    }
}

impl IdGenerator for FixedIds {
    fn next_session_id(&mut self) -> SessionId {
        self.session_ids
            .pop_front()
            .expect("fixed IDs include a session ID")
    }

    fn next_event_id(&mut self) -> EventId {
        self.event_ids
            .pop_front()
            .expect("fixed IDs include an event ID")
    }
}

struct ScriptedCapture {
    observations: VecDeque<RuntimeObservation>,
    outcome: Result<CaptureOutcome, CaptureError>,
}

impl ExecutionCapture for ScriptedCapture {
    fn capture(
        &mut self,
        _request: &CaptureRequest,
        sink: &mut dyn ObservationSink,
    ) -> Result<CaptureOutcome, CaptureError> {
        while let Some(observation) = self.observations.pop_front() {
            sink.record(observation)?;
        }
        self.outcome.clone()
    }
}

fn command() -> CommandSpec {
    CommandSpec::new(
        NativePath::from_unix_bytes(b"/usr/bin/printf".to_vec()),
        vec![NativeString::from_unix_bytes(b"hello".to_vec())],
        NativePath::from_unix_bytes(b"/workspace".to_vec()),
    )
}

fn process_source() -> ObservationSource {
    ObservationSource::new(SourceId::from_u128(7), ObservationSourceKind::Process)
}

#[test]
fn scripted_capture_records_one_ordered_session_through_the_public_seam() {
    let process_id = ProcessInstanceId::from_u128(20);
    let initial_workspace = WorkspaceState::initial();
    let observations = VecDeque::from([
        RuntimeObservation::new(
            process_source(),
            ObservationTime::wall_clock(UnixNanos::new(110)),
            RuntimeObservationKind::ProcessStarted(ProcessStarted {
                process_id,
                parent_process_id: None,
                operating_system_pid: Some(4242),
                command: command(),
                workspace_state: Some(initial_workspace.clone()),
            }),
        ),
        RuntimeObservation::recorder_gap(
            GapScope::FileSystem,
            GapReason::Unsupported,
            "scripted acceptance source does not observe files".to_owned(),
        ),
        RuntimeObservation::new(
            process_source(),
            ObservationTime::wall_clock(UnixNanos::new(120)),
            RuntimeObservationKind::ProcessExited(ProcessExited {
                process_id,
                termination: ProcessTermination::ExitCode(0),
            }),
        ),
    ]);
    let mut capture = ScriptedCapture {
        observations,
        outcome: Ok(CaptureOutcome::new(
            SessionOutcome::Completed,
            Some(initial_workspace.clone()),
        )),
    };

    let execution = record_execution(
        InMemoryEventStore::default(),
        FixedClock::new([100, 111, 112, 121, 130]),
        FixedIds::new(&[1], &[10, 11, 12, 13, 14]),
        &mut capture,
        CaptureRequest::new(command(), Some(initial_workspace)),
    )
    .expect("scripted capture records");

    assert!(execution.capture_error().is_none());
    assert_eq!(EventSequence::new(4), execution.session().final_sequence());
    let session_id = execution.session().session_id();
    let (store, _, _) = execution.into_session().into_parts();
    let events = store.events(session_id);

    assert_eq!(5, events.len());
    for (index, event) in events.iter().enumerate() {
        assert_eq!(EventSequence::new(index as u64), event.sequence());
    }
    assert!(matches!(
        events[0].observation(),
        Observation::SessionStarted(_)
    ));
    assert!(matches!(events[1].observation(), Observation::Runtime(_)));
    assert!(matches!(events[2].observation(), Observation::Runtime(_)));
    assert!(matches!(events[3].observation(), Observation::Runtime(_)));
    assert!(matches!(
        events[4].observation(),
        Observation::SessionEnded(_)
    ));
}

#[test]
fn capture_failure_is_recorded_as_a_gap_and_closes_the_session_without_a_final_state() {
    let mut capture = ScriptedCapture {
        observations: VecDeque::new(),
        outcome: Err(CaptureError::Failed("observer crashed".to_owned())),
    };

    let execution = record_execution(
        InMemoryEventStore::default(),
        FixedClock::new([100, 110, 120]),
        FixedIds::new(&[1], &[10, 11, 12]),
        &mut capture,
        CaptureRequest::new(command(), Some(WorkspaceState::initial())),
    )
    .expect("capture failure is represented in the session");

    assert_eq!(
        Some(&CaptureError::Failed("observer crashed".to_owned())),
        execution.capture_error()
    );
    let session_id = execution.session().session_id();
    let (store, _, _) = execution.into_session().into_parts();
    let events = store.events(session_id);

    assert_eq!(3, events.len());
    let Observation::SessionEnded(ended) = events[2].observation() else {
        panic!("last event must end the session");
    };
    assert_eq!(SessionOutcome::CaptureFailed, ended.outcome());
    assert_eq!(None, ended.final_workspace());
}
