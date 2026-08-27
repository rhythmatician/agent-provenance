#![forbid(unsafe_code)]
#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    clippy::single_match,
    clippy::collapsible_match
)]

use provenance_core::{
    EvidenceContinuity, EvidenceFreshness, assess_validation_freshness, classify_cargo_validation,
};
use provenance_domain::{
    EventEnvelope, GapScope, Observation, ProcessTermination, RuntimeObservationKind,
    WorkspaceState,
};

use crate::timeline::{OUTPUT_SCHEMA_VERSION, event_to_dto};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    pub session_id: provenance_domain::SessionId,
    pub final_state: Option<WorkspaceState>,
    pub last_passing: Option<PassingValidation>,
    pub freshness: EvidenceFreshness,
    pub continuity: EvidenceContinuity,
    pub has_gap_after_validation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassingValidation {
    pub kind: provenance_core::CargoValidationKind,
    pub rule_version: u32,
    pub event_id: provenance_domain::EventId,
    pub sequence: u64,
    pub validated_state: Option<WorkspaceState>,
}

pub fn build_validation_report(
    session_id: provenance_domain::SessionId,
    events: &[EventEnvelope],
) -> ValidationReport {
    let mut last_passing: Option<PassingValidation> = None;
    let mut last_passing_seq: u64 = 0;
    let mut final_state: Option<WorkspaceState> = None;
    let mut has_any_gap = false;
    let mut gap_after_last_passing = false;

    // Track workspace state at each point for validation context
    // For MVP, we consider the validated_state as the WorkspaceState at the time of the validation process's start?
    // Simpler: Use the final_state for freshness comparison, and track if any gap occurred after last passing.

    // First, find final_state from SessionEnded or last WorkspaceStateAdvanced
    for event in events {
        match event.observation() {
            Observation::SessionEnded(se) => {
                final_state = se.final_workspace().cloned();
            }
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::WorkspaceStateAdvanced(wsa) => {
                    final_state = Some(wsa.transition.current().clone());
                }
                RuntimeObservationKind::ObservationGap(gap) => {
                    has_any_gap = true;
                    if last_passing.is_some() && event.sequence().value() > last_passing_seq {
                        gap_after_last_passing = true;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // If no final_state from SessionEnded, use last WorkspaceStateAdvanced or initial
    if final_state.is_none() {
        // Try to find last WorkspaceStateAdvanced
        for event in events.iter().rev() {
            if let Observation::Runtime(rt) = event.observation() {
                if let RuntimeObservationKind::WorkspaceStateAdvanced(wsa) = rt.kind() {
                    final_state = Some(wsa.transition.current().clone());
                    break;
                }
            }
        }
    }

    // Find last passing cargo validation
    // Need to pair ProcessStarted and ProcessExited for same process_id
    use std::collections::HashMap;
    let mut started: HashMap<
        provenance_domain::ProcessInstanceId,
        (
            EventEnvelope,
            provenance_domain::CommandSpec,
            Option<WorkspaceState>,
        ),
    > = HashMap::new();

    for event in events {
        match event.observation() {
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::ProcessStarted(ps) => {
                    started.insert(
                        ps.process_id,
                        (
                            event.clone(),
                            ps.command.clone(),
                            ps.workspace_state.clone(),
                        ),
                    );
                }
                RuntimeObservationKind::ProcessExited(pe) => {
                    if let Some((start_event, command, ws)) = started.remove(&pe.process_id) {
                        if let Some(validation) =
                            classify_cargo_validation(&command, &pe.termination)
                        {
                            if validation.is_passing {
                                // Only consider passing as supporting evidence
                                // Record the last passing by sequence (the exit event's sequence)
                                if event.sequence().value() > last_passing_seq {
                                    last_passing_seq = event.sequence().value();
                                    last_passing = Some(PassingValidation {
                                        kind: validation.kind,
                                        rule_version: validation.rule_version,
                                        event_id: event.event_id(),
                                        sequence: event.sequence().value(),
                                        validated_state: ws.clone().or_else(|| final_state.clone()),
                                    });
                                    // Reset gap flag since we found a new last passing
                                    gap_after_last_passing = false;
                                }
                            }
                        }
                    }
                }
                RuntimeObservationKind::ObservationGap(gap) => {
                    if gap.scope == GapScope::WorkspaceState || gap.scope == GapScope::FileSystem {
                        has_any_gap = true;
                        if last_passing.is_some() && event.sequence().value() > last_passing_seq {
                            gap_after_last_passing = true;
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // Determine continuity: if any gap, or gap after last passing, then GapObserved
    let continuity = if has_any_gap || gap_after_last_passing {
        // For MVP, any gap makes freshness indeterminate, but we also need to handle gap after validation specifically
        // The spec says: workspace observation gap after validation makes freshness indeterminate
        // So if gap_after_last_passing, then indeterminate
        EvidenceContinuity::GapObserved
    } else {
        // Also check if there is any gap in the session at all that is WorkspaceState/FileSystem
        let has_gap = events.iter().any(|e| match e.observation() {
            Observation::Runtime(rt) => matches!(rt.kind(), RuntimeObservationKind::ObservationGap(gap) if gap.scope == GapScope::WorkspaceState || gap.scope == GapScope::FileSystem),
            _ => false,
        });
        if has_gap {
            EvidenceContinuity::GapObserved
        } else {
            EvidenceContinuity::Complete
        }
    };

    let freshness = match &last_passing {
        Some(passing) => {
            let validated_state = passing.validated_state.as_ref();
            // For stale check, compare validated_state to final_state
            // If we don't have validated_state, treat as indeterminate if no final_state, otherwise stale/current based on final_state
            match (validated_state, final_state.as_ref()) {
                (Some(vs), Some(fs)) => {
                    // If gap after, indeterminate already handled via continuity
                    if gap_after_last_passing {
                        EvidenceFreshness::Indeterminate
                    } else {
                        assess_validation_freshness(vs, Some(fs), continuity)
                    }
                }
                (None, Some(_)) => {
                    // No validated state but we have final, and no gap: treat as current if we have a passing? But without ws, we can't tell, so indeterminate
                    if continuity == EvidenceContinuity::GapObserved {
                        EvidenceFreshness::Indeterminate
                    } else {
                        // For MVP, if we have a passing validation but no ws, consider it current for now (since we can't prove stale)
                        EvidenceFreshness::Current
                    }
                }
                _ => EvidenceFreshness::Indeterminate,
            }
        }
        None => EvidenceFreshness::Indeterminate,
    };

    ValidationReport {
        session_id,
        final_state,
        last_passing,
        freshness,
        continuity,
        has_gap_after_validation: gap_after_last_passing,
    }
}

pub fn format_validation_human(report: &ValidationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("session {}\n", report.session_id));
    match &report.last_passing {
        Some(passing) => {
            out.push_str(&format!(
                "last passing: {:?} rule v{} event {} seq={}\n",
                passing.kind, passing.rule_version, passing.event_id, passing.sequence
            ));
        }
        None => {
            out.push_str("last passing: none\n");
        }
    }
    if let Some(fs) = &report.final_state {
        out.push_str(&format!(
            "final workspace generation: {}\n",
            fs.generation().value()
        ));
    } else {
        out.push_str("final workspace: none\n");
    }
    out.push_str(&format!("freshness: {:?}\n", report.freshness));
    out.push_str(&format!("continuity: {:?}\n", report.continuity));
    if report.has_gap_after_validation {
        out.push_str("gap after validation: true\n");
    }
    out
}

pub fn format_validation_json(
    report: &ValidationReport,
    events: &[EventEnvelope],
) -> Result<String, String> {
    // For JSON, we include the same report but also the filtered validation events
    // Use TimelineJson for the validation events (passing only)
    let filtered: Vec<crate::timeline::EventDto> = events
        .iter()
        .filter(|e| match e.observation() {
            Observation::Runtime(rt) => match rt.kind() {
                RuntimeObservationKind::ProcessStarted(_) => false,
                RuntimeObservationKind::ProcessExited(_) => {
                    // We need to find the corresponding start to classify, but for JSON we can just include all ProcessExited that are passing
                    // Instead, we filter by checking if the event is a passing validation's exit
                    if let Some(passing) = &report.last_passing {
                        e.event_id() == passing.event_id
                    } else {
                        false
                    }
                }
                _ => false,
            },
            _ => false,
        })
        .map(event_to_dto)
        .collect();

    #[derive(serde::Serialize)]
    struct ValidationJson {
        output_schema_version: u16,
        session_id: String,
        freshness: String,
        continuity: String,
        final_workspace_generation: Option<u64>,
        last_passing: Option<LastPassingDto>,
        events: Vec<crate::timeline::EventDto>,
    }

    #[derive(serde::Serialize)]
    struct LastPassingDto {
        kind: String,
        rule_version: u32,
        event_id: String,
        sequence: u64,
    }

    let payload = ValidationJson {
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        session_id: report.session_id.to_string(),
        freshness: format!("{:?}", report.freshness),
        continuity: format!("{:?}", report.continuity),
        final_workspace_generation: report
            .final_state
            .as_ref()
            .map(|ws| ws.generation().value()),
        last_passing: report.last_passing.as_ref().map(|p| LastPassingDto {
            kind: format!("{:?}", p.kind),
            rule_version: p.rule_version,
            event_id: p.event_id.to_string(),
            sequence: p.sequence,
        }),
        events: filtered,
    };
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}
