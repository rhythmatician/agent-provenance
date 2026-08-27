#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use provenance_core::{
    CaptureError, CaptureOutcome, CaptureRequest, ExecutionCapture, ObservationSink,
};
#[allow(unused_imports)]
use provenance_domain::{
    CommandSpec, GapReason, GapScope, NativePath, ObservationSource, ObservationSourceKind,
    ObservationTime, ProcessInstanceId, ProcessStarted, ProcessTermination, RuntimeObservation,
    RuntimeObservationKind, SessionOutcome, SourceId, UnixNanos, WorkspaceState,
};

/// Linux capture adapter for the root process only.
///
/// On Linux (including WSL) it spawns the requested command as a child,
/// records `ProcessStarted`/`ProcessExited` with lossless native paths, and
/// emits explicit `ObservationGap` events for descendant processes and
/// filesystem mutations. On other platforms `capture` returns `Unsupported`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinuxCaptureAdapter;

impl LinuxCaptureAdapter {
    #[cfg(target_os = "linux")]
    fn new_process_instance_id() -> ProcessInstanceId {
        let mut bytes = [0u8; 16];
        if getrandom::getrandom(&mut bytes).is_err() {
            // Fallback: mix time and pid if getrandom fails (should not happen on Linux)
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let pid = std::process::id() as u128;
            let mixed = nanos ^ (pid << 64);
            bytes = mixed.to_le_bytes();
        }
        ProcessInstanceId::from_u128(u128::from_le_bytes(bytes))
    }

    #[cfg(target_os = "linux")]
    fn now_unix_nanos() -> UnixNanos {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => {
                let nanos = duration.as_nanos();
                let truncated = i64::try_from(nanos).unwrap_or(i64::MAX);
                UnixNanos::new(truncated)
            }
            Err(error) => {
                let nanos = error.duration().as_nanos();
                let truncated = i64::try_from(nanos).unwrap_or(i64::MAX);
                UnixNanos::new(truncated.saturating_neg())
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn now_observation_time() -> ObservationTime {
        ObservationTime::wall_clock(Self::now_unix_nanos())
    }

    #[cfg(target_os = "linux")]
    fn native_path_to_os_string(path: &NativePath) -> OsString {
        // On Linux, NativePath is always UnixBytes; if it is Windows encoding we still
        // preserve bytes losslessly by using the raw units.
        OsString::from_vec(path.as_native_string().units().to_vec())
    }

    #[cfg(target_os = "linux")]
    fn native_string_to_os_string(value: &provenance_domain::NativeString) -> OsString {
        OsString::from_vec(value.units().to_vec())
    }
}

#[cfg(target_os = "linux")]
impl ExecutionCapture for LinuxCaptureAdapter {
    fn capture(
        &mut self,
        request: &CaptureRequest,
        sink: &mut dyn ObservationSink,
    ) -> Result<CaptureOutcome, CaptureError> {
        // Emit the two required gaps for this vertical slice. They are present in every
        // session produced by this adapter, regardless of child outcome.
        // We emit them before ProcessStarted so they are always present even if spawn fails.
        // However, to keep ordering deterministic (ProcessStarted before gaps), we will
        // record gaps right after ProcessStarted. For the error path where spawn fails,
        // we still need to emit gaps, so we do it in both places.

        let process_id = Self::new_process_instance_id();
        let source = ObservationSource::new(SourceId::from_u128(1), ObservationSourceKind::Process);
        let command = request.command().clone();
        let working_dir_os = Self::native_path_to_os_string(command.working_directory());

        // Prepare std::process::Command
        let mut cmd = Command::new(Self::native_path_to_os_string(command.executable()));
        for arg in command.arguments() {
            cmd.arg(Self::native_string_to_os_string(arg));
        }
        cmd.current_dir(Path::new(&working_dir_os));
        // Ensure we don't inherit unnecessary handles; keep stdout/stderr as parent's
        cmd.stdin(Stdio::null());
        // We intentionally do not capture output (MVP excludes output capture)
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        // Spawn the child
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                // Even on spawn failure, emit the required gaps so every session has them
                sink.record(RuntimeObservation::recorder_gap(
                    GapScope::ProcessTree,
                    GapReason::Unsupported,
                    "descendant process capture not yet implemented; root-only adapter".to_owned(),
                ))?;
                sink.record(RuntimeObservation::recorder_gap(
                    GapScope::FileSystem,
                    GapReason::Unsupported,
                    "filesystem mutation capture not yet implemented".to_owned(),
                ))?;
                return Err(CaptureError::Failed(format!(
                    "failed to spawn {}: {error}",
                    command.executable().to_string_lossy()
                )));
            }
        };

        let os_pid = child.id();

        // Record ProcessStarted with the real OS pid
        sink.record(RuntimeObservation::new(
            source,
            Self::now_observation_time(),
            RuntimeObservationKind::ProcessStarted(ProcessStarted {
                process_id,
                parent_process_id: None,
                operating_system_pid: Some(os_pid),
                command: command.clone(),
                workspace_state: request.initial_workspace().cloned(),
            }),
        ))?;

        // Emit required gaps for this slice
        sink.record(RuntimeObservation::recorder_gap(
            GapScope::ProcessTree,
            GapReason::Unsupported,
            "descendant process capture not yet implemented; root-only adapter".to_owned(),
        ))?;
        sink.record(RuntimeObservation::recorder_gap(
            GapScope::FileSystem,
            GapReason::Unsupported,
            "filesystem mutation capture not yet implemented".to_owned(),
        ))?;

        // Set up cancellation flag for SIGINT/SIGTERM
        let terminated = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "linux")]
        {
            let flag = Arc::clone(&terminated);
            let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&flag));
            let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, flag);
        }

        // Wait for child with periodic polling to handle cancellation
        let status = loop {
            if terminated.load(Ordering::Relaxed) {
                // Cancellation requested: terminate and reap the child
                let _ = child.kill();
                // Wait for the child to be reaped; use a bounded wait
                match child.wait() {
                    Ok(status) => break status,
                    Err(error) => {
                        return Err(CaptureError::Failed(format!(
                            "failed to wait after kill: {error}"
                        )));
                    }
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(CaptureError::Failed(format!(
                        "failed to wait for child: {error}"
                    )));
                }
            }
        };

        // Distinguish exit code vs signal termination (Unix only)
        let termination = {
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(code) = status.code() {
                    ProcessTermination::ExitCode(code)
                } else if let Some(signal) = status.signal() {
                    ProcessTermination::Signal(signal)
                } else {
                    ProcessTermination::Unknown
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                if let Some(code) = status.code() {
                    ProcessTermination::ExitCode(code)
                } else {
                    ProcessTermination::Unknown
                }
            }
        };

        let was_cancelled = terminated.load(Ordering::Relaxed);

        // Record ProcessExited
        sink.record(RuntimeObservation::new(
            source,
            Self::now_observation_time(),
            RuntimeObservationKind::ProcessExited(provenance_domain::ProcessExited {
                process_id,
                termination,
            }),
        ))?;

        if was_cancelled {
            // Cancellation terminates the session as Aborted
            Ok(CaptureOutcome::new(SessionOutcome::Aborted, None))
        } else {
            Ok(CaptureOutcome::new(
                SessionOutcome::Completed,
                request.initial_workspace().cloned(),
            ))
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl ExecutionCapture for LinuxCaptureAdapter {
    fn capture(
        &mut self,
        _request: &CaptureRequest,
        _sink: &mut dyn ObservationSink,
    ) -> Result<CaptureOutcome, CaptureError> {
        Err(CaptureError::Unsupported(
            "LinuxCaptureAdapter is only supported on Linux (including WSL)".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    #[allow(unused_imports)]
    use provenance_core::{
        CaptureRequest, Clock, EventStore, ExecutionCapture, IdGenerator, ObservationSink,
        SessionRecorder,
    };
    #[allow(unused_imports)]
    use provenance_domain::{
        CommandSpec, EventId, GapReason, GapScope, NativePath, Observation, RuntimeObservationKind,
        SessionId, UnixNanos,
    };

    use super::LinuxCaptureAdapter;
    use crate::memory::InMemoryEventStore;

    struct FixedClock {
        values: VecDeque<UnixNanos>,
    }

    impl FixedClock {
        fn new(values: impl IntoIterator<Item = i64>) -> Self {
            Self {
                values: values.into_iter().map(UnixNanos::new).collect(),
            }
        }
    }

    impl Clock for FixedClock {
        fn now(&mut self) -> UnixNanos {
            self.values.pop_front().expect("fixed clock")
        }
    }

    struct FixedIds {
        session_ids: VecDeque<SessionId>,
        event_ids: VecDeque<EventId>,
    }

    impl FixedIds {
        fn new(session_ids: &[u128], event_ids: &[u128]) -> Self {
            Self {
                session_ids: session_ids
                    .iter()
                    .copied()
                    .map(SessionId::from_u128)
                    .collect(),
                event_ids: event_ids.iter().copied().map(EventId::from_u128).collect(),
            }
        }
    }

    impl IdGenerator for FixedIds {
        fn next_session_id(&mut self) -> SessionId {
            self.session_ids.pop_front().expect("session id")
        }

        fn next_event_id(&mut self) -> EventId {
            self.event_ids.pop_front().expect("event id")
        }
    }

    fn command_for_bin(bin: &str) -> CommandSpec {
        CommandSpec::new(
            NativePath::from_unix_bytes(bin.as_bytes().to_vec()),
            Vec::new(),
            NativePath::from_unix_bytes(b"/tmp".to_vec()),
        )
    }

    #[test]
    fn linux_adapter_emits_gaps_on_linux() {
        // This test only verifies gap emission on Linux; on other platforms it expects Unsupported
        let mut adapter = LinuxCaptureAdapter;
        let store = InMemoryEventStore::default();
        let mut recorder = SessionRecorder::start(
            store,
            FixedClock::new([100, 101, 102, 103, 104, 105, 106]),
            FixedIds::new(&[1], &[10, 11, 12, 13, 14, 15, 16]),
            command_for_bin("/bin/true"),
            None,
        )
        .expect("start recorder");

        let request = CaptureRequest::new(command_for_bin("/bin/true"), None);
        let result = adapter.capture(&request, &mut recorder);

        #[cfg(target_os = "linux")]
        {
            assert!(
                result.is_ok(),
                "linux adapter should succeed for /bin/true: {result:?}"
            );
            let session_id = recorder.session_id();
            // Finish to get store
            let completed = recorder
                .finish(provenance_domain::SessionOutcome::Completed, None)
                .unwrap();
            let (store, _, _) = completed.into_parts();
            let events = store.events(session_id);
            // Should have SessionStarted, ProcessStarted, 2 gaps, ProcessExited, SessionEnded (via record_execution wrapper not here, but capture itself emits ProcessStarted, gaps, ProcessExited)
            // Since we called capture directly on recorder, SessionStarted was already appended at start, and ProcessStarted/gaps/ProcessExited are runtime observations
            let mut has_process_started = false;
            let mut has_process_exited = false;
            let mut gap_scopes = Vec::new();
            for event in events {
                if let Observation::Runtime(runtime) = event.observation() {
                    match runtime.kind() {
                        RuntimeObservationKind::ProcessStarted(_) => has_process_started = true,
                        RuntimeObservationKind::ProcessExited(_) => has_process_exited = true,
                        RuntimeObservationKind::ObservationGap(gap) => gap_scopes.push(gap.scope),
                        _ => {}
                    }
                }
            }
            assert!(has_process_started, "should have ProcessStarted");
            assert!(has_process_exited, "should have ProcessExited");
            assert!(
                gap_scopes.contains(&GapScope::ProcessTree),
                "should have ProcessTree gap"
            );
            assert!(
                gap_scopes.contains(&GapScope::FileSystem),
                "should have FileSystem gap"
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                provenance_core::CaptureError::Unsupported(_)
            ));
        }
    }
}
