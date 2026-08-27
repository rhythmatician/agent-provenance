use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use provenance_core::{EventStore, EventStoreError, ExpectedVersion};
use provenance_domain::{
    CommandSpec, ContentDigest, DigestAlgorithm, EVENT_SCHEMA_VERSION, EventEnvelope, EventId,
    EventSequence, FileMutationKind, FileMutationObserved, GapReason, GapScope, MonotonicNanos,
    NativeEncoding, NativePath, NativeString, Observation, ObservationGap, ObservationSource,
    ObservationSourceKind, ObservationTime, ProcessExited, ProcessInstanceId, ProcessStarted,
    ProcessTermination, RuntimeObservation, RuntimeObservationKind, SessionEnded, SessionId,
    SessionOutcome, SessionStarted, SourceId, UnixNanos, WorkspaceGeneration, WorkspaceState,
    WorkspaceStateAdvanced, WorkspaceTransition,
};

/// SQLite-backed [`EventStore`] that durably appends and reloads event envelopes.
///
/// Initialization is idempotent: `open` and `open_in_memory` create tables with
/// `IF NOT EXISTS` and can be called repeatedly on the same path.
pub struct SqliteEventStore {
    conn: Connection,
}

impl SqliteEventStore {
    /// Open an in-memory database. Useful for tests.
    pub fn open_in_memory() -> Result<Self, EventStoreError> {
        let conn = Connection::open_in_memory()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        let store = Self { conn };
        store.init()?;
        store.recover_incomplete_sessions()?;
        Ok(store)
    }

    /// Open or create a file-backed database at `path`.
    ///
    /// Parent directories are created if missing. `PRAGMA journal_mode=WAL` is
    /// set for crash safety. The call is idempotent.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, EventStoreError> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        let store = Self { conn };
        store.init()?;
        store.recover_incomplete_sessions()?;
        Ok(store)
    }

    fn init(&self) -> Result<(), EventStoreError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS events (
                    session_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL,
                    event_id TEXT NOT NULL,
                    schema_version INTEGER NOT NULL,
                    recorded_at INTEGER NOT NULL,
                    observation_json TEXT NOT NULL,
                    PRIMARY KEY (session_id, sequence)
                );
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at INTEGER NOT NULL
                );",
            )
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn recover_incomplete_sessions(&self) -> Result<(), EventStoreError> {
        // Find sessions with SessionStarted but no SessionEnded, and append recorder-restart gap + aborted end.
        // This is idempotent: if a session already has SessionEnded, we skip.
        // For MVP we treat all sessions as owned and adopt them if incomplete.
        // We need to query distinct session_ids and check each.
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT session_id FROM events")
            .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
        let session_ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| EventStoreError::Unavailable(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for session_id_text in session_ids {
            // Check if session has SessionEnded
            let has_ended: bool = {
                let mut stmt2 = self
                    .conn
                    .prepare("SELECT observation_json FROM events WHERE session_id = ?1 ORDER BY sequence")
                    .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
                let rows = stmt2
                    .query_map([session_id_text.clone()], |row| row.get::<_, String>(0))
                    .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
                let mut has = false;
                for row in rows {
                    let json = row.map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
                    if json.contains("\"type\":\"SessionEnded\"")
                        || json.contains("\"SessionEnded\"")
                    {
                        // More robust: try to deserialize to check type, but simple string check for MVP
                        // We check if the observation is SessionEnded by looking for "SessionEnded" in JSON
                        // This is fragile but works for our DTO encoding where ObservationDto::SessionEnded serializes as {"type":"SessionEnded",...}
                        if json.contains("SessionEnded") {
                            has = true;
                            break;
                        }
                    }
                }
                has
            };
            if has_ended {
                continue;
            }
            // Incomplete session: append gap and aborted SessionEnded
            // Find max sequence for this session
            let max_seq: i64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(sequence), -1) FROM events WHERE session_id = ?1",
                    [session_id_text.clone()],
                    |row| row.get(0),
                )
                .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
            let next_seq = max_seq + 1;
            // Generate new event IDs and recorded_at
            let mut id_bytes = [0u8; 16];
            if getrandom::getrandom(&mut id_bytes).is_err() {
                // Fallback to time-based
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                id_bytes = nanos.to_le_bytes()[0..16].try_into().unwrap_or([0u8; 16]);
            }
            let gap_event_id = format!("{:032x}", u128::from_le_bytes(id_bytes));
            let mut id_bytes2 = [0u8; 16];
            if getrandom::getrandom(&mut id_bytes2).is_err() {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                id_bytes2 = (nanos ^ 0x9e3779b97f4a7c15u128).to_le_bytes()[0..16]
                    .try_into()
                    .unwrap_or([0u8; 16]);
            }
            let end_event_id = format!("{:032x}", u128::from_le_bytes(id_bytes2));
            let recorded_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            // Prepare gap observation DTO
            // We need to construct JSON for RuntimeObservation with GapScope::Recorder / RecorderRestarted
            // For simplicity, we directly insert JSON strings for the two new events
            // Gap event
            let gap_observation_json = format!(
                r#"{{"type":"Runtime","payload":{{"source":{{"id":"{:032x}","kind":"Recorder"}},"observed_at":{{"wall_clock":{recorded_at},"monotonic":null}},"kind":{{"kind":"ObservationGap","payload":{{"scope":"Recorder","reason":"RecorderRestarted","detail":"recorder restarted after crash"}}}}}}}}"#,
                0u128
            );
            let end_observation_json =
                r#"{"type":"SessionEnded","payload":{"outcome":"Aborted","final_workspace":null}}"#
                    .to_owned();
            // Use a transaction for atomicity
            // For MVP we just execute two inserts; if one fails, the other may still be there, but idempotency will handle via has_ended check (now has SessionEnded, so next time skip)
            // But we should ensure we don't duplicate gap if already inserted: check if last event is already RecorderRestarted gap
            // For simplicity, we check if the last observation for this session already contains RecorderRestarted, then skip gap insertion
            let last_json: Option<String> = self
                .conn
                .query_row(
                    "SELECT observation_json FROM events WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 1",
                    [session_id_text.clone()],
                    |row| row.get(0),
                )
                .ok();
            let has_restart_gap = last_json
                .as_ref()
                .map(|s| s.contains("RecorderRestarted"))
                .unwrap_or(false);
            if !has_restart_gap {
                self.conn
                    .execute(
                        "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            session_id_text.clone(),
                            next_seq,
                            gap_event_id,
                            1,
                            recorded_at,
                            gap_observation_json
                        ],
                    )
                    .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
                self.conn
                    .execute(
                        "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            session_id_text.clone(),
                            next_seq + 1,
                            end_event_id,
                            1,
                            recorded_at + 1,
                            end_observation_json
                        ],
                    )
                    .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
            } else {
                // Already has gap, just ensure SessionEnded exists (it doesn't per has_ended check, so this branch shouldn't happen)
                // But if has_restart_gap and no SessionEnded, we still need to add SessionEnded
                self.conn
                    .execute(
                        "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            session_id_text,
                            next_seq,
                            end_event_id,
                            1,
                            recorded_at,
                            end_observation_json
                        ],
                    )
                    .map_err(|e| EventStoreError::Unavailable(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn session_id_to_text(id: SessionId) -> String {
        format!("{:032x}", id.as_u128())
    }

    fn event_id_to_text(id: EventId) -> String {
        format!("{:032x}", id.as_u128())
    }

    fn process_id_to_text(id: ProcessInstanceId) -> String {
        format!("{:032x}", id.as_u128())
    }

    fn source_id_to_text(id: SourceId) -> String {
        format!("{:032x}", id.as_u128())
    }

    fn parse_hex_u128(text: &str, field: &str) -> Result<u128, EventStoreError> {
        u128::from_str_radix(text, 16)
            .map_err(|_| EventStoreError::Corrupt(format!("invalid {field}: {text}")))
    }
}

// ---------------------------------------------------------------------------
// DTOs — versioned payload encoding, kept inside the adapter so `domain`
// remains dependency-free.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct NativeStringDto {
    encoding: String,
    bytes_base64: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct CommandSpecDto {
    executable: NativeStringDto,
    arguments: Vec<NativeStringDto>,
    working_directory: NativeStringDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ContentDigestDto {
    algorithm: String,
    bytes_base64: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct WorkspaceStateDto {
    generation: u64,
    digest: Option<ContentDigestDto>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct WorkspaceTransitionDto {
    previous: WorkspaceStateDto,
    current: WorkspaceStateDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ObservationTimeDto {
    wall_clock: Option<i64>,
    monotonic: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ObservationSourceDto {
    id: String,
    kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ProcessStartedDto {
    process_id: String,
    parent_process_id: Option<String>,
    operating_system_pid: Option<u32>,
    command: CommandSpecDto,
    workspace_state: Option<WorkspaceStateDto>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind")]
enum ProcessTerminationDto {
    ExitCode { code: i32 },
    Signal { signal: i32 },
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ProcessExitedDto {
    process_id: String,
    termination: ProcessTerminationDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind")]
enum FileMutationKindDto {
    Created,
    Modified,
    Deleted,
    Renamed { from: NativeStringDto },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct FileMutationObservedDto {
    path: NativeStringDto,
    kind: FileMutationKindDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct WorkspaceStateAdvancedDto {
    transition: WorkspaceTransitionDto,
    cause_event: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ObservationGapDto {
    scope: String,
    reason: String,
    detail: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload")]
enum RuntimeObservationKindDto {
    ProcessStarted(ProcessStartedDto),
    ProcessExited(ProcessExitedDto),
    FileMutationObserved(FileMutationObservedDto),
    WorkspaceStateAdvanced(WorkspaceStateAdvancedDto),
    ObservationGap(ObservationGapDto),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct RuntimeObservationDto {
    source: ObservationSourceDto,
    observed_at: ObservationTimeDto,
    kind: RuntimeObservationKindDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct SessionStartedDto {
    command: CommandSpecDto,
    initial_workspace: Option<WorkspaceStateDto>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct SessionEndedDto {
    outcome: String,
    final_workspace: Option<WorkspaceStateDto>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
enum ObservationDto {
    SessionStarted(SessionStartedDto),
    Runtime(RuntimeObservationDto),
    SessionEnded(SessionEndedDto),
}

// ---------------------------------------------------------------------------
// Conversion helpers domain <-> DTO
// ---------------------------------------------------------------------------

fn native_string_to_dto(value: &NativeString) -> NativeStringDto {
    let encoding = match value.encoding() {
        NativeEncoding::UnixBytes => "UnixBytes",
        NativeEncoding::WindowsWtf16LittleEndian => "WindowsWtf16LittleEndian",
    }
    .to_owned();
    NativeStringDto {
        encoding,
        bytes_base64: BASE64.encode(value.units()),
    }
}

fn dto_to_native_string(dto: NativeStringDto) -> Result<NativeString, EventStoreError> {
    let bytes = BASE64
        .decode(&dto.bytes_base64)
        .map_err(|error| EventStoreError::Corrupt(format!("base64 decode failed: {error}")))?;
    match dto.encoding.as_str() {
        "UnixBytes" => Ok(NativeString::from_unix_bytes(bytes)),
        "WindowsWtf16LittleEndian" => NativeString::from_windows_wtf16_bytes(bytes)
            .map_err(|error| EventStoreError::Corrupt(error.to_string())),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown NativeEncoding {other}"
        ))),
    }
}

fn native_path_to_dto(value: &NativePath) -> NativeStringDto {
    native_string_to_dto(value.as_native_string())
}

fn dto_to_native_path(dto: NativeStringDto) -> Result<NativePath, EventStoreError> {
    let native = dto_to_native_string(dto)?;
    match native.encoding() {
        NativeEncoding::UnixBytes => Ok(NativePath::from_unix_bytes(native.units().to_vec())),
        NativeEncoding::WindowsWtf16LittleEndian => {
            let bytes = native.units();
            if bytes.len() % 2 != 0 {
                return Err(EventStoreError::Corrupt(
                    "odd Windows byte length for NativePath".to_owned(),
                ));
            }
            let mut units = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks_exact(2) {
                units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            Ok(NativePath::from_windows_wtf16(&units))
        }
    }
}

fn command_to_dto(value: &CommandSpec) -> CommandSpecDto {
    CommandSpecDto {
        executable: native_path_to_dto(value.executable()),
        arguments: value.arguments().iter().map(native_string_to_dto).collect(),
        working_directory: native_path_to_dto(value.working_directory()),
    }
}

fn dto_to_command(dto: CommandSpecDto) -> Result<CommandSpec, EventStoreError> {
    let executable = dto_to_native_path(dto.executable)?;
    let mut arguments = Vec::with_capacity(dto.arguments.len());
    for arg in dto.arguments {
        arguments.push(dto_to_native_string(arg)?);
    }
    let working_directory = dto_to_native_path(dto.working_directory)?;
    Ok(CommandSpec::new(executable, arguments, working_directory))
}

fn digest_to_dto(value: &ContentDigest) -> ContentDigestDto {
    let algorithm = match value.algorithm() {
        DigestAlgorithm::Blake3 => "Blake3",
    }
    .to_owned();
    ContentDigestDto {
        algorithm,
        bytes_base64: BASE64.encode(value.bytes()),
    }
}

fn dto_to_digest(dto: ContentDigestDto) -> Result<ContentDigest, EventStoreError> {
    let bytes_vec = BASE64
        .decode(&dto.bytes_base64)
        .map_err(|error| EventStoreError::Corrupt(format!("base64 decode failed: {error}")))?;
    if bytes_vec.len() != 32 {
        return Err(EventStoreError::Corrupt(format!(
            "ContentDigest must be 32 bytes; got {}",
            bytes_vec.len()
        )));
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&bytes_vec);
    match dto.algorithm.as_str() {
        "Blake3" => Ok(ContentDigest::blake3(bytes)),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown DigestAlgorithm {other}"
        ))),
    }
}

fn workspace_state_to_dto(value: &WorkspaceState) -> WorkspaceStateDto {
    WorkspaceStateDto {
        generation: value.generation().value(),
        digest: value.digest().map(digest_to_dto),
    }
}

fn dto_to_workspace_state(dto: WorkspaceStateDto) -> Result<WorkspaceState, EventStoreError> {
    let digest = dto.digest.map(dto_to_digest).transpose()?;
    Ok(WorkspaceState::new(
        WorkspaceGeneration::new(dto.generation),
        digest,
    ))
}

fn transition_to_dto(value: &WorkspaceTransition) -> WorkspaceTransitionDto {
    WorkspaceTransitionDto {
        previous: workspace_state_to_dto(value.previous()),
        current: workspace_state_to_dto(value.current()),
    }
}

fn dto_to_transition(dto: WorkspaceTransitionDto) -> Result<WorkspaceTransition, EventStoreError> {
    let previous = dto_to_workspace_state(dto.previous)?;
    let current = dto_to_workspace_state(dto.current)?;
    WorkspaceTransition::new(previous, current)
        .map_err(|error| EventStoreError::Corrupt(error.to_string()))
}

fn observation_time_to_dto(value: ObservationTime) -> ObservationTimeDto {
    ObservationTimeDto {
        wall_clock: value.wall_clock_value().map(|v| v.value()),
        monotonic: value.monotonic_value().map(|v| v.value()),
    }
}

fn dto_to_observation_time(dto: ObservationTimeDto) -> ObservationTime {
    match (dto.wall_clock, dto.monotonic) {
        (Some(wall), Some(mono)) => {
            ObservationTime::both(UnixNanos::new(wall), MonotonicNanos::new(mono))
        }
        (Some(wall), None) => ObservationTime::wall_clock(UnixNanos::new(wall)),
        (None, _) => ObservationTime::unknown(),
    }
}

fn source_kind_to_str(kind: ObservationSourceKind) -> &'static str {
    match kind {
        ObservationSourceKind::Recorder => "Recorder",
        ObservationSourceKind::Process => "Process",
        ObservationSourceKind::FileSystem => "FileSystem",
        ObservationSourceKind::Workspace => "Workspace",
        ObservationSourceKind::Other => "Other",
    }
}

fn str_to_source_kind(text: &str) -> Result<ObservationSourceKind, EventStoreError> {
    match text {
        "Recorder" => Ok(ObservationSourceKind::Recorder),
        "Process" => Ok(ObservationSourceKind::Process),
        "FileSystem" => Ok(ObservationSourceKind::FileSystem),
        "Workspace" => Ok(ObservationSourceKind::Workspace),
        "Other" => Ok(ObservationSourceKind::Other),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown ObservationSourceKind {other}"
        ))),
    }
}

fn source_to_dto(value: ObservationSource) -> ObservationSourceDto {
    ObservationSourceDto {
        id: SqliteEventStore::source_id_to_text(value.id()),
        kind: source_kind_to_str(value.kind()).to_owned(),
    }
}

fn dto_to_source(dto: ObservationSourceDto) -> Result<ObservationSource, EventStoreError> {
    let id = SourceId::from_u128(SqliteEventStore::parse_hex_u128(&dto.id, "SourceId")?);
    let kind = str_to_source_kind(&dto.kind)?;
    Ok(ObservationSource::new(id, kind))
}

fn gap_scope_to_str(scope: GapScope) -> &'static str {
    match scope {
        GapScope::ProcessTree => "ProcessTree",
        GapScope::FileSystem => "FileSystem",
        GapScope::WorkspaceState => "WorkspaceState",
        GapScope::Output => "Output",
        GapScope::CaptureAdapter => "CaptureAdapter",
        GapScope::Recorder => "Recorder",
    }
}

fn str_to_gap_scope(text: &str) -> Result<GapScope, EventStoreError> {
    match text {
        "ProcessTree" => Ok(GapScope::ProcessTree),
        "FileSystem" => Ok(GapScope::FileSystem),
        "WorkspaceState" => Ok(GapScope::WorkspaceState),
        "Output" => Ok(GapScope::Output),
        "CaptureAdapter" => Ok(GapScope::CaptureAdapter),
        "Recorder" => Ok(GapScope::Recorder),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown GapScope {other}"
        ))),
    }
}

fn gap_reason_to_str(reason: GapReason) -> &'static str {
    match reason {
        GapReason::Unsupported => "Unsupported",
        GapReason::BufferOverflow => "BufferOverflow",
        GapReason::PermissionDenied => "PermissionDenied",
        GapReason::ObserverFailed => "ObserverFailed",
        GapReason::RecorderRestarted => "RecorderRestarted",
        GapReason::Unknown => "Unknown",
    }
}

fn str_to_gap_reason(text: &str) -> Result<GapReason, EventStoreError> {
    match text {
        "Unsupported" => Ok(GapReason::Unsupported),
        "BufferOverflow" => Ok(GapReason::BufferOverflow),
        "PermissionDenied" => Ok(GapReason::PermissionDenied),
        "ObserverFailed" => Ok(GapReason::ObserverFailed),
        "RecorderRestarted" => Ok(GapReason::RecorderRestarted),
        "Unknown" => Ok(GapReason::Unknown),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown GapReason {other}"
        ))),
    }
}

fn outcome_to_str(outcome: SessionOutcome) -> &'static str {
    match outcome {
        SessionOutcome::Completed => "Completed",
        SessionOutcome::Aborted => "Aborted",
        SessionOutcome::CaptureFailed => "CaptureFailed",
    }
}

fn str_to_outcome(text: &str) -> Result<SessionOutcome, EventStoreError> {
    match text {
        "Completed" => Ok(SessionOutcome::Completed),
        "Aborted" => Ok(SessionOutcome::Aborted),
        "CaptureFailed" => Ok(SessionOutcome::CaptureFailed),
        other => Err(EventStoreError::Corrupt(format!(
            "unknown SessionOutcome {other}"
        ))),
    }
}

fn observation_to_dto(observation: &Observation) -> Result<ObservationDto, EventStoreError> {
    match observation {
        Observation::SessionStarted(started) => {
            Ok(ObservationDto::SessionStarted(SessionStartedDto {
                command: command_to_dto(started.command()),
                initial_workspace: started.initial_workspace().map(workspace_state_to_dto),
            }))
        }
        Observation::SessionEnded(ended) => Ok(ObservationDto::SessionEnded(SessionEndedDto {
            outcome: outcome_to_str(ended.outcome()).to_owned(),
            final_workspace: ended.final_workspace().map(workspace_state_to_dto),
        })),
        Observation::Runtime(runtime) => {
            let kind = match runtime.kind() {
                RuntimeObservationKind::ProcessStarted(value) => {
                    RuntimeObservationKindDto::ProcessStarted(ProcessStartedDto {
                        process_id: SqliteEventStore::process_id_to_text(value.process_id),
                        parent_process_id: value
                            .parent_process_id
                            .map(SqliteEventStore::process_id_to_text),
                        operating_system_pid: value.operating_system_pid,
                        command: command_to_dto(&value.command),
                        workspace_state: value.workspace_state.as_ref().map(workspace_state_to_dto),
                    })
                }
                RuntimeObservationKind::ProcessExited(value) => {
                    RuntimeObservationKindDto::ProcessExited(ProcessExitedDto {
                        process_id: SqliteEventStore::process_id_to_text(value.process_id),
                        termination: match value.termination {
                            ProcessTermination::ExitCode(code) => {
                                ProcessTerminationDto::ExitCode { code }
                            }
                            ProcessTermination::Signal(signal) => {
                                ProcessTerminationDto::Signal { signal }
                            }
                            ProcessTermination::Unknown => ProcessTerminationDto::Unknown,
                        },
                    })
                }
                RuntimeObservationKind::FileMutationObserved(value) => {
                    RuntimeObservationKindDto::FileMutationObserved(FileMutationObservedDto {
                        path: native_path_to_dto(&value.path),
                        kind: match &value.kind {
                            FileMutationKind::Created => FileMutationKindDto::Created,
                            FileMutationKind::Modified => FileMutationKindDto::Modified,
                            FileMutationKind::Deleted => FileMutationKindDto::Deleted,
                            FileMutationKind::Renamed { from } => FileMutationKindDto::Renamed {
                                from: native_path_to_dto(from),
                            },
                        },
                    })
                }
                RuntimeObservationKind::WorkspaceStateAdvanced(value) => {
                    RuntimeObservationKindDto::WorkspaceStateAdvanced(WorkspaceStateAdvancedDto {
                        transition: transition_to_dto(&value.transition),
                        cause_event: value.cause_event.map(SqliteEventStore::event_id_to_text),
                    })
                }
                RuntimeObservationKind::ObservationGap(value) => {
                    RuntimeObservationKindDto::ObservationGap(ObservationGapDto {
                        scope: gap_scope_to_str(value.scope).to_owned(),
                        reason: gap_reason_to_str(value.reason).to_owned(),
                        detail: value.detail.clone(),
                    })
                }
            };
            Ok(ObservationDto::Runtime(RuntimeObservationDto {
                source: source_to_dto(runtime.source()),
                observed_at: observation_time_to_dto(runtime.observed_at()),
                kind,
            }))
        }
    }
}

fn dto_to_observation(dto: ObservationDto) -> Result<Observation, EventStoreError> {
    match dto {
        ObservationDto::SessionStarted(value) => {
            let command = dto_to_command(value.command)?;
            let initial_workspace = value
                .initial_workspace
                .map(dto_to_workspace_state)
                .transpose()?;
            Ok(Observation::SessionStarted(SessionStarted::new(
                command,
                initial_workspace,
            )))
        }
        ObservationDto::SessionEnded(value) => {
            let outcome = str_to_outcome(&value.outcome)?;
            let final_workspace = value
                .final_workspace
                .map(dto_to_workspace_state)
                .transpose()?;
            Ok(Observation::SessionEnded(SessionEnded::new(
                outcome,
                final_workspace,
            )))
        }
        ObservationDto::Runtime(value) => {
            let source = dto_to_source(value.source)?;
            let observed_at = dto_to_observation_time(value.observed_at);
            let kind = match value.kind {
                RuntimeObservationKindDto::ProcessStarted(dto) => {
                    let process_id = ProcessInstanceId::from_u128(
                        SqliteEventStore::parse_hex_u128(&dto.process_id, "ProcessInstanceId")?,
                    );
                    let parent_process_id = dto
                        .parent_process_id
                        .map(|text| {
                            SqliteEventStore::parse_hex_u128(&text, "ProcessInstanceId")
                                .map(ProcessInstanceId::from_u128)
                        })
                        .transpose()?;
                    let command = dto_to_command(dto.command)?;
                    let workspace_state = dto
                        .workspace_state
                        .map(dto_to_workspace_state)
                        .transpose()?;
                    RuntimeObservationKind::ProcessStarted(ProcessStarted {
                        process_id,
                        parent_process_id,
                        operating_system_pid: dto.operating_system_pid,
                        command,
                        workspace_state,
                    })
                }
                RuntimeObservationKindDto::ProcessExited(dto) => {
                    let process_id = ProcessInstanceId::from_u128(
                        SqliteEventStore::parse_hex_u128(&dto.process_id, "ProcessInstanceId")?,
                    );
                    let termination = match dto.termination {
                        ProcessTerminationDto::ExitCode { code } => {
                            ProcessTermination::ExitCode(code)
                        }
                        ProcessTerminationDto::Signal { signal } => {
                            ProcessTermination::Signal(signal)
                        }
                        ProcessTerminationDto::Unknown => ProcessTermination::Unknown,
                    };
                    RuntimeObservationKind::ProcessExited(ProcessExited {
                        process_id,
                        termination,
                    })
                }
                RuntimeObservationKindDto::FileMutationObserved(dto) => {
                    let path = dto_to_native_path(dto.path)?;
                    let kind = match dto.kind {
                        FileMutationKindDto::Created => FileMutationKind::Created,
                        FileMutationKindDto::Modified => FileMutationKind::Modified,
                        FileMutationKindDto::Deleted => FileMutationKind::Deleted,
                        FileMutationKindDto::Renamed { from } => FileMutationKind::Renamed {
                            from: dto_to_native_path(from)?,
                        },
                    };
                    RuntimeObservationKind::FileMutationObserved(FileMutationObserved {
                        path,
                        kind,
                    })
                }
                RuntimeObservationKindDto::WorkspaceStateAdvanced(dto) => {
                    let transition = dto_to_transition(dto.transition)?;
                    let cause_event = dto
                        .cause_event
                        .map(|text| {
                            SqliteEventStore::parse_hex_u128(&text, "EventId")
                                .map(EventId::from_u128)
                        })
                        .transpose()?;
                    RuntimeObservationKind::WorkspaceStateAdvanced(WorkspaceStateAdvanced {
                        transition,
                        cause_event,
                    })
                }
                RuntimeObservationKindDto::ObservationGap(dto) => {
                    let scope = str_to_gap_scope(&dto.scope)?;
                    let reason = str_to_gap_reason(&dto.reason)?;
                    RuntimeObservationKind::ObservationGap(ObservationGap {
                        scope,
                        reason,
                        detail: dto.detail,
                    })
                }
            };
            Ok(Observation::Runtime(RuntimeObservation::new(
                source,
                observed_at,
                kind,
            )))
        }
    }
}

fn serialize_observation(observation: &Observation) -> Result<String, EventStoreError> {
    let dto = observation_to_dto(observation)?;
    serde_json::to_string(&dto).map_err(|error| EventStoreError::Corrupt(error.to_string()))
}

fn deserialize_observation(json: &str) -> Result<Observation, EventStoreError> {
    let dto: ObservationDto =
        serde_json::from_str(json).map_err(|error| EventStoreError::Corrupt(error.to_string()))?;
    dto_to_observation(dto)
}

// ---------------------------------------------------------------------------
// EventStore impl
// ---------------------------------------------------------------------------

impl EventStore for SqliteEventStore {
    fn append(
        &mut self,
        expected: ExpectedVersion,
        event: EventEnvelope,
    ) -> Result<(), EventStoreError> {
        let session_text = Self::session_id_to_text(event.session_id());
        let tx = self
            .conn
            .transaction()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;

        let actual: Option<i64> = tx
            .query_row(
                "SELECT sequence FROM events WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 1",
                params![session_text],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;

        let actual_seq = actual
            .map(|value| {
                if value < 0 {
                    return Err(EventStoreError::Corrupt(format!(
                        "negative sequence {value}"
                    )));
                }
                #[allow(clippy::cast_sign_loss)]
                let unsigned = value as u64;
                Ok(EventSequence::new(unsigned))
            })
            .transpose()?;

        let version_matches = match expected {
            ExpectedVersion::Empty => actual_seq.is_none(),
            ExpectedVersion::Exact(expected_value) => actual_seq == Some(expected_value),
        };
        if !version_matches {
            return Err(EventStoreError::Conflict {
                expected,
                actual: actual_seq,
            });
        }

        let expected_sequence = match actual_seq {
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

        let observation_json = serialize_observation(event.observation())?;
        let event_id_text = Self::event_id_to_text(event.event_id());
        let recorded_at = event.recorded_at().value();
        let schema_version = i64::from(event.schema_version());
        let sequence_value = i64::try_from(event.sequence().value())
            .map_err(|_| EventStoreError::Corrupt("sequence does not fit in i64".to_owned()))?;

        tx.execute(
            "INSERT INTO events (session_id, sequence, event_id, schema_version, recorded_at, observation_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_text,
                sequence_value,
                event_id_text,
                schema_version,
                recorded_at,
                observation_json
            ],
        )
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("UNIQUE") || message.contains("PRIMARY") {
                EventStoreError::Corrupt(format!("duplicate sequence insert: {message}"))
            } else {
                EventStoreError::Unavailable(message)
            }
        })?;

        tx.commit()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn load(&self, session_id: SessionId) -> Result<Vec<EventEnvelope>, EventStoreError> {
        let session_text = Self::session_id_to_text(session_id);
        let mut statement = self
            .conn
            .prepare(
                "SELECT sequence, event_id, schema_version, recorded_at, observation_json FROM events WHERE session_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;

        let rows = statement
            .query_map(params![session_text], |row| {
                let sequence: i64 = row.get(0)?;
                let event_id: String = row.get(1)?;
                let schema_version: i64 = row.get(2)?;
                let recorded_at: i64 = row.get(3)?;
                let observation_json: String = row.get(4)?;
                Ok((
                    sequence,
                    event_id,
                    schema_version,
                    recorded_at,
                    observation_json,
                ))
            })
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;

        let mut events = Vec::new();
        for row_result in rows {
            let (
                sequence_value,
                event_id_text,
                schema_version_value,
                recorded_at_value,
                observation_json,
            ) = row_result.map_err(|error| EventStoreError::Corrupt(error.to_string()))?;

            if sequence_value < 0 {
                return Err(EventStoreError::Corrupt(format!(
                    "negative sequence {sequence_value}"
                )));
            }
            #[allow(clippy::cast_sign_loss)]
            let sequence = EventSequence::new(sequence_value as u64);
            let event_id = EventId::from_u128(Self::parse_hex_u128(&event_id_text, "EventId")?);
            let schema_version = u16::try_from(schema_version_value).map_err(|_| {
                EventStoreError::Corrupt(format!("invalid schema_version {schema_version_value}"))
            })?;
            if schema_version != EVENT_SCHEMA_VERSION {
                return Err(EventStoreError::Corrupt(format!(
                    "unsupported schema_version {schema_version}"
                )));
            }
            let recorded_at = UnixNanos::new(recorded_at_value);
            let observation = deserialize_observation(&observation_json)?;

            let envelope =
                EventEnvelope::new(event_id, session_id, sequence, recorded_at, observation);
            events.push(envelope);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use provenance_core::{
        CaptureError, CaptureOutcome, CaptureRequest, Clock, EventStore, EventStoreError,
        ExecutionCapture, ExpectedVersion, IdGenerator, ObservationSink, record_execution,
    };
    use provenance_domain::{
        CommandSpec, EventEnvelope, EventId, EventSequence, GapReason, GapScope, NativePath,
        NativeString, Observation, ObservationSource, ObservationSourceKind, ObservationTime,
        ProcessInstanceId, ProcessStarted, ProcessTermination, RuntimeObservation,
        RuntimeObservationKind, SessionId, SessionOutcome, SessionStarted, SourceId, UnixNanos,
        WorkspaceState,
    };

    use super::SqliteEventStore;

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
        let mut store = SqliteEventStore::open_in_memory().expect("open in memory");
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

    #[test]
    fn wrong_expected_version_does_not_append() {
        let mut store = SqliteEventStore::open_in_memory().expect("open in memory");
        store
            .append(ExpectedVersion::Empty, started_event(EventSequence::ZERO))
            .expect("first event appends");

        let _ = store.append(ExpectedVersion::Empty, started_event(EventSequence::new(1)));

        let events = store.load(SessionId::from_u128(1)).expect("load succeeds");
        assert_eq!(1, events.len());
        assert_eq!(EventSequence::ZERO, events[0].sequence());
    }

    #[test]
    fn invalid_sequence_does_not_append_partial() {
        let mut store = SqliteEventStore::open_in_memory().expect("open in memory");
        store
            .append(ExpectedVersion::Empty, started_event(EventSequence::ZERO))
            .expect("first event appends");

        let wrong_seq_event = EventEnvelope::new(
            EventId::from_u128(99),
            SessionId::from_u128(1),
            EventSequence::new(5),
            UnixNanos::new(10),
            Observation::SessionStarted(SessionStarted::new(
                CommandSpec::new(
                    NativePath::from_unix_bytes(b"echo".to_vec()),
                    Vec::new(),
                    NativePath::from_unix_bytes(b"/tmp".to_vec()),
                ),
                None,
            )),
        );
        let result = store.append(ExpectedVersion::Exact(EventSequence::ZERO), wrong_seq_event);
        assert!(matches!(
            result,
            Err(EventStoreError::InvalidSequence { .. })
        ));

        let events = store.load(SessionId::from_u128(1)).expect("load succeeds");
        assert_eq!(1, events.len());
    }

    #[test]
    fn initialization_is_idempotent() {
        let mut store = SqliteEventStore::open_in_memory().expect("open in memory");
        store.init().expect("second init succeeds");
        store.init().expect("third init succeeds");

        store
            .append(ExpectedVersion::Empty, started_event(EventSequence::ZERO))
            .expect("append after idempotent init");
        let events = store.load(SessionId::from_u128(1)).expect("load succeeds");
        assert_eq!(1, events.len());
    }

    #[test]
    fn reopening_file_preserves_events_and_is_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "provenance-test-reopen-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let mut store = SqliteEventStore::open(&path).expect("open file");
            store
                .append(ExpectedVersion::Empty, started_event(EventSequence::ZERO))
                .expect("append");
        }
        {
            let store = SqliteEventStore::open(&path).expect("reopen file");
            store.init().expect("init idempotent on reopen");
            let events = store
                .load(SessionId::from_u128(1))
                .expect("load after reopen");
            // After ticket 8, incomplete sessions are recovered with a gap and Aborted end on open
            assert_eq!(3, events.len());
            assert_eq!(EventSequence::ZERO, events[0].sequence());
            assert_eq!(EventSequence::new(1), events[1].sequence());
            assert_eq!(EventSequence::new(2), events[2].sequence());
            match events[1].observation() {
                Observation::Runtime(rt) => match rt.kind() {
                    RuntimeObservationKind::ObservationGap(gap) => {
                        assert_eq!(GapScope::Recorder, gap.scope);
                        assert_eq!(GapReason::RecorderRestarted, gap.reason);
                    }
                    other => panic!("expected recorder gap, got {other:?}"),
                },
                other => panic!("expected runtime gap, got {other:?}"),
            }
            match events[2].observation() {
                Observation::SessionEnded(se) => {
                    assert_eq!(provenance_domain::SessionOutcome::Aborted, se.outcome());
                }
                other => panic!("expected SessionEnded Aborted, got {other:?}"),
            }
        }
        {
            let store = SqliteEventStore::open(&path).expect("third open");
            let events = store.load(SessionId::from_u128(1)).expect("load final");
            // Recovery is idempotent: third open still has 3 events, not 5
            assert_eq!(3, events.len());
            assert_eq!(EventSequence::ZERO, events[0].sequence());
            assert_eq!(EventSequence::new(2), events[2].sequence());
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

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
    fn session_recorded_via_public_seam_is_reloadable_with_identical_fields() {
        let path = std::env::temp_dir().join(format!(
            "provenance-test-roundtrip-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);

        let session_id;
        let expected_events: Vec<EventEnvelope>;
        {
            let store = SqliteEventStore::open(&path).expect("open file");
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
                CaptureRequest::new(command(), Some(initial_workspace)),
            )
            .expect("record_execution succeeds");

            assert!(execution.capture_error().is_none());
            session_id = execution.session().session_id();
            let (store_after, _, _) = execution.into_session().into_parts();
            expected_events = store_after.load(session_id).expect("load from same store");
            assert_eq!(5, expected_events.len());
        }

        {
            let store = SqliteEventStore::open(&path).expect("reopen for reload");
            let reloaded = store.load(session_id).expect("load from new connection");
            assert_eq!(expected_events.len(), reloaded.len());
            for (expected, actual) in expected_events.iter().zip(reloaded.iter()) {
                assert_eq!(expected.event_id(), actual.event_id());
                assert_eq!(expected.session_id(), actual.session_id());
                assert_eq!(expected.sequence(), actual.sequence());
                assert_eq!(expected.recorded_at(), actual.recorded_at());
                assert_eq!(expected.observation(), actual.observation());
                assert_eq!(expected.schema_version(), actual.schema_version());
            }
        }

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn non_utf8_paths_round_trip() {
        let mut store = SqliteEventStore::open_in_memory().expect("open in memory");
        let session = SessionId::from_u128(42);
        let event = EventEnvelope::new(
            EventId::from_u128(1),
            session,
            EventSequence::ZERO,
            UnixNanos::new(1),
            Observation::SessionStarted(SessionStarted::new(
                CommandSpec::new(
                    NativePath::from_unix_bytes(vec![0x66, 0x80, 0x6f]),
                    vec![NativeString::from_unix_bytes(vec![0xff, 0xfe])],
                    NativePath::from_unix_bytes(vec![0x00, 0xff]),
                ),
                None,
            )),
        );
        store
            .append(ExpectedVersion::Empty, event.clone())
            .expect("append non-utf8");
        let loaded = store.load(session).expect("load");
        assert_eq!(1, loaded.len());
        assert_eq!(event.observation(), loaded[0].observation());
        assert_eq!(event.event_id(), loaded[0].event_id());
    }
}
