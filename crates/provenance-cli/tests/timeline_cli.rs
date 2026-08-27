#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::process::Command;

use provenance_adapters::SqliteEventStore;
use provenance_core::{
    CaptureError, CaptureOutcome, CaptureRequest, Clock, EventStore, ExecutionCapture, IdGenerator,
    ObservationSink, record_execution,
};
use provenance_domain::{
    CommandSpec, EventId, GapReason, GapScope, NativePath, NativeString, ObservationSource,
    ObservationSourceKind, ObservationTime, ProcessInstanceId, ProcessStarted, ProcessTermination,
    RuntimeObservation, RuntimeObservationKind, SessionId, SessionOutcome, SourceId, UnixNanos,
    WorkspaceState,
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

fn command_spec() -> CommandSpec {
    CommandSpec::new(
        NativePath::from_unix_bytes(b"/usr/bin/printf".to_vec()),
        vec![NativeString::from_unix_bytes(b"hello".to_vec())],
        NativePath::from_unix_bytes(b"/workspace".to_vec()),
    )
}

fn process_source() -> ObservationSource {
    ObservationSource::new(SourceId::from_u128(7), ObservationSourceKind::Process)
}

fn create_fixture_db(path: &std::path::Path) -> SessionId {
    let store = SqliteEventStore::open(path).expect("open fixture db");
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
                command: command_spec(),
                workspace_state: Some(initial_workspace.clone()),
            }),
        ),
        RuntimeObservation::recorder_gap(
            GapScope::FileSystem,
            GapReason::Unsupported,
            "scripted source does not observe files".to_owned(),
        ),
        RuntimeObservation::new(
            process_source(),
            ObservationTime::wall_clock(UnixNanos::new(120)),
            RuntimeObservationKind::ProcessExited(provenance_domain::ProcessExited {
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
        store,
        FixedClock::new([100, 111, 112, 121, 130]),
        FixedIds::new(&[1], &[10, 11, 12, 13, 14]),
        &mut capture,
        CaptureRequest::new(command_spec(), Some(initial_workspace)),
    )
    .expect("record_execution succeeds");
    let session_id = execution.session().session_id();
    // Steal store via into_parts to ensure it was written, then drop
    let (store_after, _, _) = execution.into_session().into_parts();
    let events = store_after.load(session_id).expect("load after record");
    assert_eq!(5, events.len());
    // Drop store_after is already moved, but we created a new store via open(path) earlier that is still at `store`? Actually we moved store into record_execution, so the DB file is now closed when execution is dropped? The store_after is the same connection, but we need to ensure file is flushed. Dropping store_after will close.
    drop(store_after);
    session_id
}

fn binary() -> std::path::PathBuf {
    // CARGO_BIN_EXE_provenance is set for integration tests
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_provenance"))
}

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("provenance-cli-test-{name}-{pid}-{nanos}.db"))
}

#[test]
fn fixture_db_is_queryable_in_fresh_process_human_and_json() {
    let path = temp_db_path("fixture");
    let _ = std::fs::remove_file(&path);
    let session_id = create_fixture_db(&path);
    let session_hex = format!("{:032x}", session_id.as_u128());

    // Human format (default)
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&session_hex)
        .arg("--db")
        .arg(&path)
        .output()
        .expect("spawn timeline human");
    assert_eq!(
        0,
        output.status.code().unwrap(),
        "human timeline should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 SessionStarted"),
        "human should contain sequence 0 SessionStarted, got: {stdout}"
    );
    assert!(
        stdout.contains("1 Runtime:ProcessStarted"),
        "human should contain ProcessStarted, got: {stdout}"
    );
    assert!(
        stdout.contains("source=Process:"),
        "human should contain source, got: {stdout}"
    );
    assert!(
        stdout.contains("2 Runtime:ObservationGap"),
        "human should contain gap, got: {stdout}"
    );
    assert!(
        stdout.contains("scope=FileSystem"),
        "human gap scope, got: {stdout}"
    );
    assert!(
        stdout.contains("reason=Unsupported"),
        "human gap reason, got: {stdout}"
    );

    // JSON format via --format json
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&session_hex)
        .arg("--db")
        .arg(&path)
        .arg("--format")
        .arg("json")
        .output()
        .expect("spawn timeline json");
    assert_eq!(
        0,
        output.status.code().unwrap(),
        "json timeline should succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("json should parse");
    assert_eq!(
        1,
        value["output_schema_version"].as_u64().unwrap(),
        "json should have output_schema_version 1"
    );
    assert_eq!(session_hex, value["session_id"].as_str().unwrap());
    let events = value["events"].as_array().unwrap();
    assert_eq!(5, events.len());
    // Lossless native-value representation: check executable encoding/bytes
    let exe = &events[0]["observation"]["payload"]["command"]["executable"];
    assert_eq!("UnixBytes", exe["encoding"].as_str().unwrap());
    assert!(
        exe["bytes_base64"].as_str().is_some(),
        "native bytes should be base64"
    );

    // Also test --json shorthand
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&session_hex)
        .arg("--db")
        .arg(&path)
        .arg("--json")
        .output()
        .expect("spawn timeline --json");
    assert_eq!(0, output.status.code().unwrap());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value2: serde_json::Value = serde_json::from_str(&stdout).expect("json2 parse");
    assert_eq!(value, value2);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn unknown_session_and_corrupt_rows_fail_with_distinct_errors() {
    let path = temp_db_path("unknown-corrupt");
    let _ = std::fs::remove_file(&path);
    let session_id = create_fixture_db(&path);
    let unknown_hex = format!("{:032x}", SessionId::from_u128(999).as_u128());

    // Unknown session should exit 3
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&unknown_hex)
        .arg("--db")
        .arg(&path)
        .output()
        .expect("spawn unknown");
    assert_eq!(
        3,
        output.status.code().unwrap(),
        "unknown session should exit 3, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown session"),
        "stderr should mention unknown session"
    );

    // Corrupt a row: directly insert invalid JSON via rusqlite
    {
        let conn = rusqlite::Connection::open(&path).expect("open for corrupt");
        conn.execute(
            "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                format!("{:032x}", session_id.as_u128()),
                99,
                format!("{:032x}", EventId::from_u128(999).as_u128()),
                1,
                999,
                "not-json{{{"
            ],
        )
        .expect("insert corrupt");
    }

    let session_hex = format!("{:032x}", session_id.as_u128());
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&session_hex)
        .arg("--db")
        .arg(&path)
        .output()
        .expect("spawn corrupt");
    assert_eq!(
        4,
        output.status.code().unwrap(),
        "corrupt row should exit 4, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("corrupt"),
        "stderr should mention corrupt"
    );
    // Ensure distinct from unknown
    assert_ne!(3, 4);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn corrupt_row_via_env_var_is_also_detected() {
    let path = temp_db_path("env-corrupt");
    let _ = std::fs::remove_file(&path);
    let session_id = create_fixture_db(&path);
    {
        let conn = rusqlite::Connection::open(&path).expect("open for corrupt2");
        conn.execute(
            "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                format!("{:032x}", session_id.as_u128()),
                100,
                format!("{:032x}", EventId::from_u128(1000).as_u128()),
                1,
                1000,
                "invalid json"
            ],
        )
        .expect("insert corrupt2");
    }
    let session_hex = format!("{:032x}", session_id.as_u128());
    let output = Command::new(binary())
        .arg("timeline")
        .arg(&session_hex)
        .env("PROVENANCE_DB", &path)
        .output()
        .expect("spawn with env var");
    assert_eq!(
        4,
        output.status.code().unwrap(),
        "env var corrupt should exit 4"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
