use provenance_domain::WorkspaceState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceContinuity {
    Complete,
    GapObserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceFreshness {
    Current,
    Stale,
    Indeterminate,
}

pub fn assess_validation_freshness(
    validated_state: &WorkspaceState,
    current_state: Option<&WorkspaceState>,
    continuity: EvidenceContinuity,
) -> EvidenceFreshness {
    if continuity == EvidenceContinuity::GapObserved {
        return EvidenceFreshness::Indeterminate;
    }

    match current_state {
        Some(current) if current == validated_state => EvidenceFreshness::Current,
        Some(_) => EvidenceFreshness::Stale,
        None => EvidenceFreshness::Indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use provenance_domain::WorkspaceState;

    use super::{EvidenceContinuity, EvidenceFreshness, assess_validation_freshness};

    #[test]
    fn validation_for_the_current_state_is_current() {
        let state = WorkspaceState::initial();

        assert_eq!(
            EvidenceFreshness::Current,
            assess_validation_freshness(&state, Some(&state), EvidenceContinuity::Complete)
        );
    }

    #[test]
    fn later_workspace_state_makes_validation_stale() {
        let validated = WorkspaceState::initial();
        let current = validated.advance(None).expect("state advances");

        assert_eq!(
            EvidenceFreshness::Stale,
            assess_validation_freshness(&validated, Some(&current), EvidenceContinuity::Complete)
        );
    }

    #[test]
    fn observation_gap_makes_freshness_indeterminate() {
        let state = WorkspaceState::initial();

        assert_eq!(
            EvidenceFreshness::Indeterminate,
            assess_validation_freshness(&state, Some(&state), EvidenceContinuity::GapObserved)
        );
    }
}

/// Rule version for cargo validation classification.
pub const CARGO_VALIDATION_RULE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CargoValidationKind {
    Test,
    Clippy,
    Check,
    FmtCheck,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoValidation {
    pub kind: CargoValidationKind,
    pub rule_version: u32,
    pub command: provenance_domain::CommandSpec,
    pub is_passing: bool,
}

/// Classify a cargo command as validation evidence if it matches supported shapes.
/// Supported: `cargo test`, `cargo clippy`, `cargo check`, `cargo fmt --check`.
/// Returns None for unknown commands (ordinary process observations).
pub fn classify_cargo_validation(
    command: &provenance_domain::CommandSpec,
    termination: &provenance_domain::ProcessTermination,
) -> Option<CargoValidation> {
    let exe_lossy = command.executable().to_string_lossy().to_lowercase();
    let is_cargo = exe_lossy == "cargo"
        || exe_lossy.ends_with("/cargo")
        || exe_lossy.ends_with("\\cargo")
        || exe_lossy.ends_with("cargo.exe");
    if !is_cargo {
        return None;
    }
    let args: Vec<String> = command
        .arguments()
        .iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    if args.is_empty() {
        return None;
    }
    let kind = match args[0].as_str() {
        "test" => CargoValidationKind::Test,
        "clippy" => CargoValidationKind::Clippy,
        "check" => CargoValidationKind::Check,
        "fmt" if args.iter().any(|a| a == "--check") => CargoValidationKind::FmtCheck,
        "fmt" => return None,
        _ => return None,
    };
    let is_passing = matches!(
        termination,
        provenance_domain::ProcessTermination::ExitCode(0)
    );
    Some(CargoValidation {
        kind,
        rule_version: CARGO_VALIDATION_RULE_VERSION,
        command: command.clone(),
        is_passing,
    })
}
