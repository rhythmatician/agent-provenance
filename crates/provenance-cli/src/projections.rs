#![forbid(unsafe_code)]

use provenance_domain::{EventEnvelope, GapScope, Observation, RuntimeObservationKind};

use crate::timeline::{OUTPUT_SCHEMA_VERSION, TimelineJson, event_to_dto};

/// Filter events for processes projection: ProcessStarted, ProcessExited, and ProcessTree gaps
pub fn filter_processes(events: &[EventEnvelope]) -> Vec<&EventEnvelope> {
    events
        .iter()
        .filter(|e| match e.observation() {
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::ProcessStarted(_)
                | RuntimeObservationKind::ProcessExited(_) => true,
                RuntimeObservationKind::ObservationGap(gap) => gap.scope == GapScope::ProcessTree,
                _ => false,
            },
            _ => false,
        })
        .collect()
}

/// Filter events for changes projection: FileMutationObserved and FileSystem gaps
pub fn filter_changes(events: &[EventEnvelope]) -> Vec<&EventEnvelope> {
    events
        .iter()
        .filter(|e| match e.observation() {
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::FileMutationObserved(_) => true,
                RuntimeObservationKind::ObservationGap(gap) => gap.scope == GapScope::FileSystem,
                _ => false,
            },
            _ => false,
        })
        .collect()
}

/// Filter events for state projection: WorkspaceStateAdvanced and WorkspaceState gaps
pub fn filter_state(events: &[EventEnvelope]) -> Vec<&EventEnvelope> {
    events
        .iter()
        .filter(|e| match e.observation() {
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::WorkspaceStateAdvanced(_) => true,
                RuntimeObservationKind::ObservationGap(gap) => {
                    gap.scope == GapScope::WorkspaceState
                }
                _ => false,
            },
            Observation::SessionEnded(_) => true,
            _ => false,
        })
        .collect()
}

pub fn format_processes_human(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> String {
    let filtered = filter_processes(events);
    let mut out = String::new();
    out.push_str(&format!("session {session_id}\n"));
    out.push_str(&format!("processes: {} events\n", filtered.len()));
    for event in filtered {
        let dto = event_to_dto(event);
        out.push_str(&format!(
            "  {} seq={} {}\n",
            dto.event_id,
            dto.sequence,
            human_summary_for_event(event)
        ));
    }
    out
}

pub fn format_changes_human(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> String {
    let filtered = filter_changes(events);
    let mut out = String::new();
    out.push_str(&format!("session {session_id}\n"));
    out.push_str(&format!("changes: {} events\n", filtered.len()));
    for event in filtered {
        let dto = event_to_dto(event);
        out.push_str(&format!(
            "  {} seq={} {}\n",
            dto.event_id,
            dto.sequence,
            human_summary_for_event(event)
        ));
    }
    out
}

pub fn format_state_human(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> String {
    let filtered = filter_state(events);
    let mut out = String::new();
    out.push_str(&format!("session {session_id}\n"));
    out.push_str(&format!("state: {} events\n", filtered.len()));
    for event in filtered {
        let dto = event_to_dto(event);
        out.push_str(&format!(
            "  {} seq={} {}\n",
            dto.event_id,
            dto.sequence,
            human_summary_for_event(event)
        ));
    }
    out
}

fn human_summary_for_event(event: &EventEnvelope) -> String {
    match event.observation() {
        Observation::Runtime(rt) => match rt.kind() {
            RuntimeObservationKind::ProcessStarted(ps) => {
                format!(
                    "ProcessStarted id={} parent={:?} pid={:?} exe={}",
                    ps.process_id,
                    ps.parent_process_id.map(|id| id.to_string()),
                    ps.operating_system_pid,
                    ps.command.executable().to_string_lossy()
                )
            }
            RuntimeObservationKind::ProcessExited(pe) => {
                format!("ProcessExited id={} {:?}", pe.process_id, pe.termination)
            }
            RuntimeObservationKind::FileMutationObserved(fm) => {
                format!("FileMutation {:?} {}", fm.kind, fm.path.to_string_lossy())
            }
            RuntimeObservationKind::WorkspaceStateAdvanced(wsa) => {
                format!(
                    "WorkspaceStateAdvanced {} -> {}",
                    wsa.transition.previous().generation().value(),
                    wsa.transition.current().generation().value()
                )
            }
            RuntimeObservationKind::ObservationGap(gap) => {
                format!("Gap {:?} {:?} {}", gap.scope, gap.reason, gap.detail)
            }
        },
        Observation::SessionStarted(ss) => {
            format!(
                "SessionStarted exe={}",
                ss.command().executable().to_string_lossy()
            )
        }
        Observation::SessionEnded(se) => {
            format!("SessionEnded {:?}", se.outcome())
        }
    }
}

pub fn format_processes_json(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> Result<String, String> {
    let filtered: Vec<crate::timeline::EventDto> = filter_processes(events)
        .into_iter()
        .map(event_to_dto)
        .collect();
    let payload = TimelineJson {
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        events: filtered,
    };
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}

pub fn format_changes_json(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> Result<String, String> {
    let filtered: Vec<crate::timeline::EventDto> = filter_changes(events)
        .into_iter()
        .map(event_to_dto)
        .collect();
    let payload = TimelineJson {
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        events: filtered,
    };
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}

pub fn format_state_json(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> Result<String, String> {
    let filtered: Vec<crate::timeline::EventDto> =
        filter_state(events).into_iter().map(event_to_dto).collect();
    let payload = TimelineJson {
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        events: filtered,
    };
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}
