#![forbid(unsafe_code)]

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

use provenance_adapters::SqliteEventStore;
use provenance_core::{EventStore, EventStoreError};
use provenance_domain::{
    CommandSpec, ContentDigest, DigestAlgorithm, EventEnvelope, FileMutationKind, GapReason,
    GapScope, NativeEncoding, NativePath, NativeString, Observation, ObservationSourceKind,
    ProcessTermination, SessionId, SessionOutcome, WorkspaceState, WorkspaceTransition,
};

pub const OUTPUT_SCHEMA_VERSION: u16 = 1;

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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EventDto {
    schema_version: u16,
    event_id: String,
    session_id: String,
    sequence: u64,
    recorded_at: i64,
    observation: ObservationDto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TimelineJson {
    pub output_schema_version: u16,
    pub session_id: String,
    pub events: Vec<EventDto>,
}

// ---------------------------------------------------------------------------
// Conversion helpers domain -> DTO (lossless, base64 for bytes, hex for ids)
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

fn native_path_to_dto(value: &NativePath) -> NativeStringDto {
    native_string_to_dto(value.as_native_string())
}

fn command_to_dto(value: &CommandSpec) -> CommandSpecDto {
    CommandSpecDto {
        executable: native_path_to_dto(value.executable()),
        arguments: value.arguments().iter().map(native_string_to_dto).collect(),
        working_directory: native_path_to_dto(value.working_directory()),
    }
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

fn workspace_state_to_dto(value: &WorkspaceState) -> WorkspaceStateDto {
    WorkspaceStateDto {
        generation: value.generation().value(),
        digest: value.digest().map(digest_to_dto),
    }
}

fn transition_to_dto(value: &WorkspaceTransition) -> WorkspaceTransitionDto {
    WorkspaceTransitionDto {
        previous: workspace_state_to_dto(value.previous()),
        current: workspace_state_to_dto(value.current()),
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

fn outcome_to_str(outcome: SessionOutcome) -> &'static str {
    match outcome {
        SessionOutcome::Completed => "Completed",
        SessionOutcome::Aborted => "Aborted",
        SessionOutcome::CaptureFailed => "CaptureFailed",
    }
}

fn observation_to_dto(observation: &Observation) -> ObservationDto {
    match observation {
        Observation::SessionStarted(started) => ObservationDto::SessionStarted(SessionStartedDto {
            command: command_to_dto(started.command()),
            initial_workspace: started.initial_workspace().map(workspace_state_to_dto),
        }),
        Observation::SessionEnded(ended) => ObservationDto::SessionEnded(SessionEndedDto {
            outcome: outcome_to_str(ended.outcome()).to_owned(),
            final_workspace: ended.final_workspace().map(workspace_state_to_dto),
        }),
        Observation::Runtime(runtime) => {
            let kind = match runtime.kind() {
                provenance_domain::RuntimeObservationKind::ProcessStarted(value) => {
                    RuntimeObservationKindDto::ProcessStarted(ProcessStartedDto {
                        process_id: format!("{:032x}", value.process_id.as_u128()),
                        parent_process_id: value
                            .parent_process_id
                            .map(|id| format!("{:032x}", id.as_u128())),
                        operating_system_pid: value.operating_system_pid,
                        command: command_to_dto(&value.command),
                        workspace_state: value.workspace_state.as_ref().map(workspace_state_to_dto),
                    })
                }
                provenance_domain::RuntimeObservationKind::ProcessExited(value) => {
                    RuntimeObservationKindDto::ProcessExited(ProcessExitedDto {
                        process_id: format!("{:032x}", value.process_id.as_u128()),
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
                provenance_domain::RuntimeObservationKind::FileMutationObserved(value) => {
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
                provenance_domain::RuntimeObservationKind::WorkspaceStateAdvanced(value) => {
                    RuntimeObservationKindDto::WorkspaceStateAdvanced(WorkspaceStateAdvancedDto {
                        transition: transition_to_dto(&value.transition),
                        cause_event: value.cause_event.map(|id| format!("{:032x}", id.as_u128())),
                    })
                }
                provenance_domain::RuntimeObservationKind::ObservationGap(value) => {
                    RuntimeObservationKindDto::ObservationGap(ObservationGapDto {
                        scope: gap_scope_to_str(value.scope).to_owned(),
                        reason: gap_reason_to_str(value.reason).to_owned(),
                        detail: value.detail.clone(),
                    })
                }
            };
            ObservationDto::Runtime(RuntimeObservationDto {
                source: ObservationSourceDto {
                    id: format!("{:032x}", runtime.source().id().as_u128()),
                    kind: source_kind_to_str(runtime.source().kind()).to_owned(),
                },
                observed_at: ObservationTimeDto {
                    wall_clock: runtime.observed_at().wall_clock_value().map(|v| v.value()),
                    monotonic: runtime.observed_at().monotonic_value().map(|v| v.value()),
                },
                kind,
            })
        }
    }
}

fn event_to_dto(envelope: &EventEnvelope) -> EventDto {
    EventDto {
        schema_version: envelope.schema_version(),
        event_id: format!("{:032x}", envelope.event_id().as_u128()),
        session_id: format!("{:032x}", envelope.session_id().as_u128()),
        sequence: envelope.sequence().value(),
        recorded_at: envelope.recorded_at().value(),
        observation: observation_to_dto(envelope.observation()),
    }
}

pub fn format_json(
    session_id: SessionId,
    events: &[EventEnvelope],
) -> Result<String, serde_json::Error> {
    let output = TimelineJson {
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        session_id: format!("{:032x}", session_id.as_u128()),
        events: events.iter().map(event_to_dto).collect(),
    };
    serde_json::to_string_pretty(&output)
}

pub fn format_human(session_id: SessionId, events: &[EventEnvelope]) -> String {
    #![allow(clippy::format_in_format_args)]
    let mut out = String::new();
    out.push_str(&format!(
        "session {:032x} timeline (output_schema_version {}, events {})\n",
        session_id.as_u128(),
        OUTPUT_SCHEMA_VERSION,
        events.len()
    ));
    for envelope in events {
        let seq = envelope.sequence().value();
        let recorded_at = envelope.recorded_at().value();
        let event_id = format!("{:032x}", envelope.event_id().as_u128());
        match envelope.observation() {
            Observation::SessionStarted(started) => {
                let cmd = started.command();
                out.push_str(&format!(
                    "{seq} SessionStarted event_id={event_id} recorded_at={recorded_at} executable={} args={} workdir={} initial_workspace={:?}\n",
                    cmd.executable().to_string_lossy(),
                    cmd.arguments()
                        .iter()
                        .map(|a| a.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(","),
                    cmd.working_directory().to_string_lossy(),
                    started
                        .initial_workspace()
                        .map(|s| format!("gen={}", s.generation().value()))
                        .unwrap_or_else(|| "None".to_owned())
                ));
            }
            Observation::SessionEnded(ended) => {
                out.push_str(&format!(
                    "{seq} SessionEnded event_id={event_id} recorded_at={recorded_at} outcome={} final_workspace={:?}\n",
                    outcome_to_str(ended.outcome()),
                    ended
                        .final_workspace()
                        .map(|s| format!("gen={}", s.generation().value()))
                        .unwrap_or_else(|| "None".to_owned())
                ));
            }
            Observation::Runtime(runtime) => {
                let source = runtime.source();
                let source_str = format!(
                    "{}:{:032x}",
                    source_kind_to_str(source.kind()),
                    source.id().as_u128()
                );
                let observed_at = runtime.observed_at();
                let time_str = match (
                    observed_at.wall_clock_value(),
                    observed_at.monotonic_value(),
                ) {
                    (Some(wall), Some(mono)) => {
                        format!("wall_clock={} monotonic={}", wall.value(), mono.value())
                    }
                    (Some(wall), None) => format!("wall_clock={}", wall.value()),
                    (None, Some(mono)) => format!("monotonic={}", mono.value()),
                    (None, None) => "unknown".to_owned(),
                };
                match runtime.kind() {
                    provenance_domain::RuntimeObservationKind::ProcessStarted(value) => {
                        out.push_str(&format!(
                            "{seq} Runtime:ProcessStarted event_id={event_id} recorded_at={recorded_at} source={source_str} observed_at={time_str} pid={:032x} parent={:?} os_pid={:?} executable={} \n",
                            value.process_id.as_u128(),
                            value
                                .parent_process_id
                                .map(|id| format!("{:032x}", id.as_u128())),
                            value.operating_system_pid,
                            value.command.executable().to_string_lossy()
                        ));
                    }
                    provenance_domain::RuntimeObservationKind::ProcessExited(value) => {
                        let term = match value.termination {
                            ProcessTermination::ExitCode(code) => format!("ExitCode({code})"),
                            ProcessTermination::Signal(sig) => format!("Signal({sig})"),
                            ProcessTermination::Unknown => "Unknown".to_owned(),
                        };
                        out.push_str(&format!(
                            "{seq} Runtime:ProcessExited event_id={event_id} recorded_at={recorded_at} source={source_str} observed_at={time_str} pid={:032x} termination={term}\n",
                            value.process_id.as_u128()
                        ));
                    }
                    provenance_domain::RuntimeObservationKind::FileMutationObserved(value) => {
                        let kind_str = match &value.kind {
                            FileMutationKind::Created => "Created".to_owned(),
                            FileMutationKind::Modified => "Modified".to_owned(),
                            FileMutationKind::Deleted => "Deleted".to_owned(),
                            FileMutationKind::Renamed { from } => {
                                format!("Renamed from {}", from.to_string_lossy())
                            }
                        };
                        out.push_str(&format!(
                            "{seq} Runtime:FileMutationObserved event_id={event_id} recorded_at={recorded_at} source={source_str} observed_at={time_str} path={} kind={kind_str}\n",
                            value.path.to_string_lossy()
                        ));
                    }
                    provenance_domain::RuntimeObservationKind::WorkspaceStateAdvanced(value) => {
                        out.push_str(&format!(
                            "{seq} Runtime:WorkspaceStateAdvanced event_id={event_id} recorded_at={recorded_at} source={source_str} observed_at={time_str} previous_gen={} current_gen={} cause_event={:?}\n",
                            value.transition.previous().generation().value(),
                            value.transition.current().generation().value(),
                            value.cause_event.map(|id| format!("{:032x}", id.as_u128()))
                        ));
                    }
                    provenance_domain::RuntimeObservationKind::ObservationGap(value) => {
                        out.push_str(&format!(
                            "{seq} Runtime:ObservationGap event_id={event_id} recorded_at={recorded_at} source={source_str} observed_at={time_str} scope={} reason={} detail={}\n",
                            gap_scope_to_str(value.scope),
                            gap_reason_to_str(value.reason),
                            value.detail
                        ));
                    }
                }
            }
        }
    }
    out
}

pub fn load_events(
    db_path: &Path,
    session_id: SessionId,
) -> Result<Vec<EventEnvelope>, EventStoreError> {
    let store = SqliteEventStore::open(db_path)?;
    store.load(session_id)
}

pub fn resolve_db_path(explicit: Option<&Path>) -> std::path::PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Ok(env_path) = std::env::var("PROVENANCE_DB") {
        if !env_path.is_empty() {
            return std::path::PathBuf::from(env_path);
        }
    }
    std::path::PathBuf::from(".provenance/provenance.db")
}

pub fn parse_session_id(text: &str) -> Result<SessionId, String> {
    let trimmed = text.trim();
    // Allow 32-char hex (with or without 0x) or decimal u128? Use hex.
    let hex = if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        &trimmed[2..]
    } else {
        trimmed
    };
    if hex.len() != 32 {
        return Err(format!(
            "session id must be 32 hex chars, got {} chars",
            hex.len()
        ));
    }
    let value =
        u128::from_str_radix(hex, 16).map_err(|_| format!("invalid session id hex: {text}"))?;
    Ok(SessionId::from_u128(value))
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use provenance_domain::{
        CommandSpec, EventEnvelope, EventId, EventSequence, GapReason, GapScope, NativePath,
        NativeString, Observation, ObservationSource, ObservationSourceKind, ObservationTime,
        ProcessInstanceId, ProcessStarted, RuntimeObservation, RuntimeObservationKind, SessionId,
        SessionStarted, SourceId, UnixNanos, WorkspaceState,
    };

    use super::{OUTPUT_SCHEMA_VERSION, format_human, format_json, parse_session_id};

    fn make_events() -> Vec<EventEnvelope> {
        let session = SessionId::from_u128(1);
        let cmd = CommandSpec::new(
            NativePath::from_unix_bytes(b"/usr/bin/printf".to_vec()),
            vec![NativeString::from_unix_bytes(b"hello".to_vec())],
            NativePath::from_unix_bytes(b"/workspace".to_vec()),
        );
        vec![
            EventEnvelope::new(
                EventId::from_u128(10),
                session,
                EventSequence::ZERO,
                UnixNanos::new(100),
                Observation::SessionStarted(SessionStarted::new(cmd.clone(), None)),
            ),
            EventEnvelope::new(
                EventId::from_u128(11),
                session,
                EventSequence::new(1),
                UnixNanos::new(111),
                Observation::Runtime(RuntimeObservation::new(
                    ObservationSource::new(SourceId::from_u128(7), ObservationSourceKind::Process),
                    ObservationTime::wall_clock(UnixNanos::new(110)),
                    RuntimeObservationKind::ProcessStarted(ProcessStarted {
                        process_id: ProcessInstanceId::from_u128(20),
                        parent_process_id: None,
                        operating_system_pid: Some(4242),
                        command: cmd,
                        workspace_state: Some(WorkspaceState::initial()),
                    }),
                )),
            ),
            EventEnvelope::new(
                EventId::from_u128(12),
                session,
                EventSequence::new(2),
                UnixNanos::new(112),
                Observation::Runtime(RuntimeObservation::recorder_gap(
                    GapScope::FileSystem,
                    GapReason::Unsupported,
                    "gap detail".to_owned(),
                )),
            ),
        ]
    }

    #[test]
    fn human_preserves_sequence_kind_source_and_gap() {
        let session = SessionId::from_u128(1);
        let events = make_events();
        let human = format_human(session, &events);
        assert!(human.contains("0 SessionStarted"));
        assert!(human.contains("1 Runtime:ProcessStarted"));
        assert!(human.contains("source=Process:"));
        assert!(human.contains("2 Runtime:ObservationGap"));
        assert!(human.contains("scope=FileSystem"));
        assert!(human.contains("reason=Unsupported"));
        assert!(human.contains("gap detail"));
    }

    #[test]
    fn json_has_output_schema_version_and_lossless_native() {
        let session = SessionId::from_u128(1);
        let mut events = make_events();
        // Add non-UTF8 path
        events.push(EventEnvelope::new(
            EventId::from_u128(13),
            session,
            EventSequence::new(3),
            UnixNanos::new(120),
            Observation::SessionStarted(SessionStarted::new(
                CommandSpec::new(
                    NativePath::from_unix_bytes(vec![0x66, 0x80, 0x6f]),
                    vec![NativeString::from_unix_bytes(vec![0xff, 0xfe])],
                    NativePath::from_unix_bytes(vec![0x00, 0xff]),
                ),
                None,
            )),
        ));
        let json = format_json(session, &events).expect("json");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse json");
        assert_eq!(
            OUTPUT_SCHEMA_VERSION as u64,
            value["output_schema_version"].as_u64().unwrap()
        );
        assert_eq!(
            "00000000000000000000000000000001",
            value["session_id"].as_str().unwrap()
        );
        let events_json = value["events"].as_array().unwrap();
        assert_eq!(4, events_json.len());
        // Check native bytes round-trip via base64
        let exe = &events_json[3]["observation"]["payload"]["command"]["executable"];
        assert_eq!("UnixBytes", exe["encoding"].as_str().unwrap());
        let bytes_b64 = exe["bytes_base64"].as_str().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(bytes_b64)
            .unwrap();
        assert_eq!(vec![0x66, 0x80, 0x6f], decoded);
    }

    #[test]
    fn parse_session_id_validates() {
        assert_eq!(
            SessionId::from_u128(1),
            parse_session_id("00000000000000000000000000000001").unwrap()
        );
        assert!(parse_session_id("nothex").is_err());
        assert!(parse_session_id("0001").is_err());
    }
}
