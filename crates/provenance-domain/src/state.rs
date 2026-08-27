use core::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceGeneration(u64);

impl WorkspaceGeneration {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithm {
    Blake3,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentDigest {
    algorithm: DigestAlgorithm,
    bytes: [u8; 32],
}

impl ContentDigest {
    pub const fn blake3(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: DigestAlgorithm::Blake3,
            bytes,
        }
    }

    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    pub const fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceState {
    generation: WorkspaceGeneration,
    digest: Option<ContentDigest>,
}

impl WorkspaceState {
    pub const fn initial() -> Self {
        Self {
            generation: WorkspaceGeneration::ZERO,
            digest: None,
        }
    }

    pub fn new(generation: WorkspaceGeneration, digest: Option<ContentDigest>) -> Self {
        Self { generation, digest }
    }

    pub const fn generation(&self) -> WorkspaceGeneration {
        self.generation
    }

    pub fn digest(&self) -> Option<&ContentDigest> {
        self.digest.as_ref()
    }

    pub fn advance(&self, digest: Option<ContentDigest>) -> Result<Self, StateError> {
        let generation = self
            .generation
            .checked_next()
            .ok_or(StateError::GenerationExhausted)?;
        Ok(Self { generation, digest })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceTransition {
    previous: WorkspaceState,
    current: WorkspaceState,
}

impl WorkspaceTransition {
    pub fn new(previous: WorkspaceState, current: WorkspaceState) -> Result<Self, StateError> {
        let expected = previous
            .generation()
            .checked_next()
            .ok_or(StateError::GenerationExhausted)?;
        if current.generation() != expected {
            return Err(StateError::NonAdjacentGeneration {
                previous: previous.generation(),
                current: current.generation(),
            });
        }
        Ok(Self { previous, current })
    }

    pub fn previous(&self) -> &WorkspaceState {
        &self.previous
    }

    pub fn current(&self) -> &WorkspaceState {
        &self.current
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    GenerationExhausted,
    NonAdjacentGeneration {
        previous: WorkspaceGeneration,
        current: WorkspaceGeneration,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationExhausted => write!(formatter, "workspace generation is exhausted"),
            Self::NonAdjacentGeneration { previous, current } => write!(
                formatter,
                "workspace transition must advance exactly once; previous={}, current={}",
                previous.value(),
                current.value()
            ),
        }
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod tests {
    use super::{StateError, WorkspaceGeneration, WorkspaceState, WorkspaceTransition};

    #[test]
    fn workspace_transition_requires_adjacent_generations() {
        let previous = WorkspaceState::initial();
        let current = WorkspaceState::new(WorkspaceGeneration::new(2), None);

        let result = WorkspaceTransition::new(previous, current);

        assert_eq!(
            Err(StateError::NonAdjacentGeneration {
                previous: WorkspaceGeneration::ZERO,
                current: WorkspaceGeneration::new(2),
            }),
            result
        );
    }

    #[test]
    fn advancing_a_workspace_state_increments_generation_once() {
        let current = WorkspaceState::initial()
            .advance(None)
            .expect("state advances");

        assert_eq!(WorkspaceGeneration::new(1), current.generation());
    }
}
