use provenance_domain::UnixNanos;

pub trait Clock {
    fn now(&mut self) -> UnixNanos;
}
