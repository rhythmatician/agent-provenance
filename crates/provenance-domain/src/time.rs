#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnixNanos(i64);

impl UnixNanos {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MonotonicNanos(u64);

impl MonotonicNanos {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationTime {
    wall_clock: Option<UnixNanos>,
    monotonic: Option<MonotonicNanos>,
}

impl ObservationTime {
    pub const fn unknown() -> Self {
        Self {
            wall_clock: None,
            monotonic: None,
        }
    }

    pub const fn wall_clock(wall_clock: UnixNanos) -> Self {
        Self {
            wall_clock: Some(wall_clock),
            monotonic: None,
        }
    }

    pub const fn both(wall_clock: UnixNanos, monotonic: MonotonicNanos) -> Self {
        Self {
            wall_clock: Some(wall_clock),
            monotonic: Some(monotonic),
        }
    }

    pub const fn wall_clock_value(self) -> Option<UnixNanos> {
        self.wall_clock
    }

    pub const fn monotonic_value(self) -> Option<MonotonicNanos> {
        self.monotonic
    }
}
