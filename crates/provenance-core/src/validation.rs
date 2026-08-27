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
