#![deny(unsafe_op_in_unsafe_fn)]

pub mod clock;
pub mod memory;
pub mod platform;

pub use clock::SystemClock;
pub use memory::InMemoryEventStore;
