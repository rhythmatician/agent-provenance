use std::time::{SystemTime, UNIX_EPOCH};

use provenance_core::Clock;
use provenance_domain::UnixNanos;

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&mut self) -> UnixNanos {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => UnixNanos::new(nanos_to_i64(duration.as_nanos())),
            Err(error) => {
                let magnitude = nanos_to_i64(error.duration().as_nanos());
                UnixNanos::new(magnitude.saturating_neg())
            }
        }
    }
}

fn nanos_to_i64(value: u128) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
