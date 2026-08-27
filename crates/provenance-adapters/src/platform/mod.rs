//! Operating-system capture implementations live here and must emit portable domain observations.
//!
//! The structs are deliberate compile-time placeholders. They do not implement `ExecutionCapture`
//! until each adapter has behavior-level acceptance tests for descendant processes, file mutation
//! coverage, loss reporting, cancellation, and cleanup.

pub mod linux;
pub mod linux_process;
pub mod windows;
