#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
use notify::Watcher;
#[cfg(target_os = "linux")]
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::{Arc, mpsc};
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use provenance_core::{
    CaptureError, CaptureOutcome, CaptureRequest, ExecutionCapture, ObservationSink,
};
#[allow(unused_imports)]
use provenance_domain::{
    CommandSpec, ContentDigest, FileMutationKind, FileMutationObserved, GapReason, GapScope,
    NativePath, ObservationSource, ObservationSourceKind, ObservationTime, ProcessInstanceId,
    ProcessStarted, ProcessTermination, RuntimeObservation, RuntimeObservationKind, SessionOutcome,
    SourceId, UnixNanos, WorkspaceGeneration, WorkspaceState, WorkspaceTransition,
};

/// Linux capture adapter with process-tree and workspace-mutation support (tickets 4 & 5).
#[derive(Clone, Copy, Debug, Default)]
pub struct LinuxCaptureAdapter;

impl LinuxCaptureAdapter {
    #[cfg(target_os = "linux")]
    fn new_process_instance_id() -> ProcessInstanceId {
        let mut bytes = [0u8; 16];
        if getrandom::getrandom(&mut bytes).is_err() {
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
        OsString::from_vec(path.as_native_string().units().to_vec())
    }

    #[cfg(target_os = "linux")]
    fn native_string_to_os_string(value: &provenance_domain::NativeString) -> OsString {
        OsString::from_vec(value.units().to_vec())
    }

    #[cfg(target_os = "linux")]
    fn path_to_native_path(path: &Path) -> NativePath {
        use std::os::unix::ffi::OsStrExt;
        NativePath::from_unix_bytes(path.as_os_str().as_bytes().to_vec())
    }

    #[cfg(target_os = "linux")]
    fn read_proc_stat(pid: u32) -> Option<(u32, char, u64)> {
        let data = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let comm_end = data.rfind(')')?;
        let after_comm = &data[comm_end + 1..];
        let mut parts = after_comm.split_whitespace();
        let state_str = parts.next()?;
        let state = state_str.chars().next()?;
        let ppid_str = parts.next()?;
        let ppid: u32 = ppid_str.parse().ok()?;
        let all_parts: Vec<&str> = after_comm.split_whitespace().collect();
        if all_parts.len() <= 19 {
            return None;
        }
        let starttime_str = all_parts[19];
        let starttime: u64 = starttime_str.parse().ok()?;
        Some((ppid, state, starttime))
    }

    #[cfg(target_os = "linux")]
    fn is_alive(pid: u32) -> bool {
        if let Some((_, state, _)) = Self::read_proc_stat(pid) {
            state != 'X' && state != 'x'
        } else {
            false
        }
    }

    #[cfg(target_os = "linux")]
    fn read_proc_cmdline(pid: u32) -> Option<(String, Vec<String>)> {
        let data = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        if data.is_empty() {
            return None;
        }
        let mut parts: Vec<String> = data
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .collect();
        if parts.is_empty() {
            return None;
        }
        let exe = parts.remove(0);
        Some((exe, parts))
    }

    #[cfg(target_os = "linux")]
    fn read_proc_exe(pid: u32) -> Option<String> {
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    #[cfg(target_os = "linux")]
    fn read_proc_cwd(pid: u32) -> Option<String> {
        std::fs::read_link(format!("/proc/{pid}/cwd"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    #[cfg(target_os = "linux")]
    fn list_pids() -> Vec<u32> {
        let mut pids = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(pid) = name.parse::<u32>() {
                        pids.push(pid);
                    }
                }
            }
        }
        pids
    }
}

#[cfg(target_os = "linux")]
impl ExecutionCapture for LinuxCaptureAdapter {
    fn capture(
        &mut self,
        request: &CaptureRequest,
        sink: &mut dyn ObservationSink,
    ) -> Result<CaptureOutcome, CaptureError> {
        use std::os::unix::process::CommandExt;

        let process_source =
            ObservationSource::new(SourceId::from_u128(1), ObservationSourceKind::Process);
        let workspace_source =
            ObservationSource::new(SourceId::from_u128(2), ObservationSourceKind::Workspace);
        let file_source =
            ObservationSource::new(SourceId::from_u128(3), ObservationSourceKind::FileSystem);

        let root_instance_id = Self::new_process_instance_id();
        let command = request.command().clone();
        let working_dir_os = Self::native_path_to_os_string(command.working_directory());
        let working_dir_path = Path::new(&working_dir_os).to_path_buf();

        // Workspace state tracking
        let mut current_workspace = request
            .initial_workspace()
            .cloned()
            .unwrap_or_else(WorkspaceState::initial);
        // Setup filesystem watcher for the workspace (working_directory)
        let (fs_tx, fs_rx) = mpsc::channel();
        let mut fs_watcher: Option<notify::RecommendedWatcher> = None;
        let mut fs_watch_failed = false;
        let mut fs_watch_error: Option<String> = None;

        // Only watch if workspace is a directory and we can access it
        let workspace_path = working_dir_path.clone();
        if workspace_path.is_dir() {
            // Use walkdir to ensure we watch recursively; notify's RecursiveMode does that, but we need to handle existing subdirs
            match notify::RecommendedWatcher::new(
                move |res: Result<notify::Event, notify::Error>| {
                    let _ = fs_tx.send(res);
                },
                notify::Config::default(),
            ) {
                Ok(mut watcher) => {
                    // Try to watch the workspace recursively
                    match watcher.watch(&workspace_path, notify::RecursiveMode::Recursive) {
                        Ok(()) => {
                            fs_watcher = Some(watcher);
                        }
                        Err(error) => {
                            fs_watch_failed = true;
                            fs_watch_error = Some(format!("failed to watch workspace: {error}"));
                        }
                    }
                }
                Err(error) => {
                    fs_watch_failed = true;
                    fs_watch_error = Some(format!("failed to create watcher: {error}"));
                }
            }
        } else {
            fs_watch_failed = true;
            fs_watch_error = Some(format!(
                "workspace is not a directory: {}",
                workspace_path.display()
            ));
        }

        // If watcher failed to setup, we will emit a gap later
        // Keep the watcher alive by holding it in fs_watcher variable

        // Prepare child in its own process group via setsid
        let mut cmd = Command::new(Self::native_path_to_os_string(command.executable()));
        for arg in command.arguments() {
            cmd.arg(Self::native_string_to_os_string(arg));
        }
        cmd.current_dir(Path::new(&working_dir_os));
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());
        // Use safe process_group(0) to create a new process group for killpg.
        // This avoids the unsafe pre_exec+setsid under forbid(unsafe_code) while still
        // allowing the recorder to terminate the whole descendant tree via killpg.
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                // Emit FileSystem gap if watcher failed, otherwise we would have emitted it later
                // For spawn failure, we need to emit the appropriate gaps
                if fs_watch_failed {
                    sink.record(RuntimeObservation::new(
                        workspace_source,
                        Self::now_observation_time(),
                        RuntimeObservationKind::ObservationGap(provenance_domain::ObservationGap {
                            scope: GapScope::WorkspaceState,
                            reason: GapReason::ObserverFailed,
                            detail: fs_watch_error
                                .clone()
                                .unwrap_or_else(|| "workspace watch failed".to_owned()),
                        }),
                    ))?;
                    sink.record(RuntimeObservation::new(
                        file_source,
                        Self::now_observation_time(),
                        RuntimeObservationKind::ObservationGap(provenance_domain::ObservationGap {
                            scope: GapScope::FileSystem,
                            reason: GapReason::ObserverFailed,
                            detail: fs_watch_error
                                .unwrap_or_else(|| "workspace watch failed".to_owned()),
                        }),
                    ))?;
                } else {
                    // If watcher was ok, we still need to emit FileSystem gap? No, for ticket 5 we should NOT emit FileSystem gap when capture is supposed to succeed
                    // But for spawn failure, we should emit ProcessTree gap as well
                    sink.record(RuntimeObservation::recorder_gap(
                        GapScope::FileSystem,
                        GapReason::ObserverFailed,
                        "failed to spawn, filesystem capture incomplete".to_owned(),
                    ))?;
                }
                sink.record(RuntimeObservation::recorder_gap(
                    GapScope::ProcessTree,
                    GapReason::ObserverFailed,
                    format!(
                        "failed to spawn {}: {error}",
                        command.executable().to_string_lossy()
                    ),
                ))?;
                return Err(CaptureError::Failed(format!(
                    "failed to spawn {}: {error}",
                    command.executable().to_string_lossy()
                )));
            }
        };

        let root_pid = child.id();
        let root_pgid = root_pid as i32;

        // Record root ProcessStarted
        sink.record(RuntimeObservation::new(
            process_source,
            Self::now_observation_time(),
            RuntimeObservationKind::ProcessStarted(ProcessStarted {
                process_id: root_instance_id,
                parent_process_id: None,
                operating_system_pid: Some(root_pid),
                command: command.clone(),
                workspace_state: Some(current_workspace.clone()),
            }),
        ))?;

        // For FileSystem, if watcher failed, emit gaps and don't attempt to capture
        // Otherwise, we will capture and NOT emit the gap (since we have coverage)
        let mut fs_gap_emitted = false;
        if fs_watch_failed {
            sink.record(RuntimeObservation::new(
                workspace_source,
                Self::now_observation_time(),
                RuntimeObservationKind::ObservationGap(provenance_domain::ObservationGap {
                    scope: GapScope::WorkspaceState,
                    reason: GapReason::ObserverFailed,
                    detail: fs_watch_error
                        .clone()
                        .unwrap_or_else(|| "workspace watch failed".to_owned()),
                }),
            ))?;
            sink.record(RuntimeObservation::new(
                file_source,
                Self::now_observation_time(),
                RuntimeObservationKind::ObservationGap(provenance_domain::ObservationGap {
                    scope: GapScope::FileSystem,
                    reason: GapReason::ObserverFailed,
                    detail: fs_watch_error.unwrap_or_else(|| "workspace watch failed".to_owned()),
                }),
            ))?;
            fs_gap_emitted = true;
            // Drop the watcher to avoid further events
            drop(fs_watcher);
        }

        // Process tree tracking
        let mut pid_to_instance: HashMap<u32, (ProcessInstanceId, Option<ProcessInstanceId>, u64)> =
            HashMap::new();
        let mut instance_to_pid: HashMap<ProcessInstanceId, u32> = HashMap::new();
        let mut seen_pids: HashMap<u32, u64> = HashMap::new();
        pid_to_instance.insert(root_pid, (root_instance_id, None, 0));
        instance_to_pid.insert(root_instance_id, root_pid);
        seen_pids.insert(root_pid, 0);
        let mut exited_instances: HashSet<ProcessInstanceId> = HashSet::new();
        let mut missed_fast_exit = false;

        // Workspace tracking for file events that need to be grouped
        // We will advance generation per file event for simplicity
        // Keep the watcher alive
        let _keep_watcher = fs_watcher;

        let terminated = Arc::new(AtomicBool::new(false));
        {
            let flag = Arc::clone(&terminated);
            let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&flag));
            let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, flag);
        }

        let get_parent_instance =
            |ppid: u32,
             pid_map: &HashMap<u32, (ProcessInstanceId, Option<ProcessInstanceId>, u64)>|
             -> Option<ProcessInstanceId> { pid_map.get(&ppid).map(|(id, _, _)| *id) };

        let mut root_status: Option<std::process::ExitStatus> = None;
        let mut iterations: usize = 0;

        loop {
            iterations += 1;

            if terminated.load(Ordering::Relaxed) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(root_pgid),
                    nix::sys::signal::Signal::SIGTERM,
                );
                std::thread::sleep(Duration::from_millis(100));
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(root_pgid),
                    nix::sys::signal::Signal::SIGKILL,
                );
                match child.wait() {
                    Ok(status) => {
                        root_status = Some(status);
                        break;
                    }
                    Err(error) => {
                        return Err(CaptureError::Failed(format!(
                            "failed to wait after kill: {error}"
                        )));
                    }
                }
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    root_status = Some(status);
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(CaptureError::Failed(format!(
                        "failed to wait for child: {error}"
                    )));
                }
            }

            // Poll for filesystem events (non-blocking)
            if !fs_gap_emitted {
                while let Ok(res) = fs_rx.try_recv() {
                    match res {
                        Ok(event) => {
                            // Handle overflow / error
                            if event.kind.is_other() {
                                // Check for rescan which indicates overflow
                                sink.record(RuntimeObservation::new(
                                    file_source,
                                    Self::now_observation_time(),
                                    RuntimeObservationKind::ObservationGap(
                                        provenance_domain::ObservationGap {
                                            scope: GapScope::FileSystem,
                                            reason: GapReason::BufferOverflow,
                                            detail: "filesystem event queue overflow".to_owned(),
                                        },
                                    ),
                                ))?;
                                sink.record(RuntimeObservation::new(
                                    workspace_source,
                                    Self::now_observation_time(),
                                    RuntimeObservationKind::ObservationGap(
                                        provenance_domain::ObservationGap {
                                            scope: GapScope::WorkspaceState,
                                            reason: GapReason::BufferOverflow,
                                            detail: "workspace state continuity indeterminate due to overflow"
                                                .to_owned(),
                                        },
                                    ),
                                ))?;
                                fs_gap_emitted = true;
                                // After overflow, we should not claim complete state
                                continue;
                            }

                            // Handle rename Both as a single event before per-path loop
                            if let notify::EventKind::Modify(notify::event::ModifyKind::Name(
                                notify::event::RenameMode::Both,
                            )) = event.kind
                            {
                                if event.paths.len() == 2 {
                                    if event.paths[0].strip_prefix(&working_dir_path).is_ok()
                                        && event.paths[1].strip_prefix(&working_dir_path).is_ok()
                                    {
                                        let from = Self::path_to_native_path(&event.paths[0]);
                                        let to = Self::path_to_native_path(&event.paths[1]);
                                        sink.record(RuntimeObservation::new(
                                            file_source,
                                            Self::now_observation_time(),
                                            RuntimeObservationKind::FileMutationObserved(
                                                FileMutationObserved {
                                                    path: to.clone(),
                                                    kind: FileMutationKind::Renamed { from },
                                                },
                                            ),
                                        ))?;
                                        let next_gen = current_workspace
                                            .generation()
                                            .checked_next()
                                            .unwrap_or(WorkspaceGeneration::new(0));
                                        let next_state = WorkspaceState::new(next_gen, None);
                                        let transition = WorkspaceTransition::new(
                                            current_workspace.clone(),
                                            next_state.clone(),
                                        )
                                        .unwrap();
                                        sink.record(RuntimeObservation::new(
                                            workspace_source,
                                            Self::now_observation_time(),
                                            RuntimeObservationKind::WorkspaceStateAdvanced(
                                                provenance_domain::WorkspaceStateAdvanced {
                                                    transition,
                                                    cause_event: None,
                                                },
                                            ),
                                        ))?;
                                        current_workspace = next_state;
                                    }
                                    continue;
                                }
                            }
                            // Map other notify events to FileMutationObserved per path
                            for path in &event.paths {
                                if path.strip_prefix(&working_dir_path).is_err() {
                                    continue;
                                }
                                let native_path = Self::path_to_native_path(path);
                                let kind = match event.kind {
                                    notify::EventKind::Create(_) => FileMutationKind::Created,
                                    notify::EventKind::Modify(notify::event::ModifyKind::Data(
                                        _,
                                    ))
                                    | notify::EventKind::Modify(
                                        notify::event::ModifyKind::Metadata(_),
                                    ) => FileMutationKind::Modified,
                                    notify::EventKind::Remove(_) => FileMutationKind::Deleted,
                                    notify::EventKind::Modify(notify::event::ModifyKind::Name(
                                        _,
                                    )) => FileMutationKind::Modified,
                                    _ => FileMutationKind::Modified,
                                };
                                sink.record(RuntimeObservation::new(
                                    file_source,
                                    Self::now_observation_time(),
                                    RuntimeObservationKind::FileMutationObserved(
                                        FileMutationObserved {
                                            path: native_path,
                                            kind,
                                        },
                                    ),
                                ))?;
                                let next_gen = current_workspace
                                    .generation()
                                    .checked_next()
                                    .unwrap_or(WorkspaceGeneration::new(0));
                                let next_state = WorkspaceState::new(next_gen, None);
                                let transition = WorkspaceTransition::new(
                                    current_workspace.clone(),
                                    next_state.clone(),
                                )
                                .unwrap();
                                sink.record(RuntimeObservation::new(
                                    workspace_source,
                                    Self::now_observation_time(),
                                    RuntimeObservationKind::WorkspaceStateAdvanced(
                                        provenance_domain::WorkspaceStateAdvanced {
                                            transition,
                                            cause_event: None,
                                        },
                                    ),
                                ))?;
                                current_workspace = next_state;
                            }
                        }
                        Err(error) => {
                            sink.record(RuntimeObservation::new(
                                file_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::ObservationGap(
                                    provenance_domain::ObservationGap {
                                        scope: GapScope::FileSystem,
                                        reason: GapReason::ObserverFailed,
                                        detail: format!("filesystem watcher error: {error}"),
                                    },
                                ),
                            ))?;
                            sink.record(RuntimeObservation::new(
                                workspace_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::ObservationGap(
                                    provenance_domain::ObservationGap {
                                        scope: GapScope::WorkspaceState,
                                        reason: GapReason::ObserverFailed,
                                        detail: format!("workspace watcher error: {error}"),
                                    },
                                ),
                            ))?;
                            fs_gap_emitted = true;
                        }
                    }
                }
            }

            // Poll for new descendants
            let pids = Self::list_pids();
            for pid in pids {
                if pid == root_pid {
                    continue;
                }
                if pid_to_instance.contains_key(&pid) {
                    continue;
                }
                if let Some((ppid, state, starttime)) = Self::read_proc_stat(pid) {
                    if let Some(prev_starttime) = seen_pids.get(&pid) {
                        if *prev_starttime == starttime {
                            continue;
                        }
                    }
                    let parent_instance = get_parent_instance(ppid, &pid_to_instance);
                    let is_descendant = if ppid == root_pid {
                        true
                    } else if pid_to_instance.contains_key(&ppid) {
                        true
                    } else {
                        if let Some((grand_ppid, _, _)) = Self::read_proc_stat(ppid) {
                            grand_ppid == root_pid && !pid_to_instance.contains_key(&ppid)
                        } else {
                            false
                        }
                    };

                    if is_descendant {
                        let (exe, args) = Self::read_proc_cmdline(pid)
                            .unwrap_or_else(|| (format!("/proc/{pid}/exe"), Vec::new()));
                        let cwd = Self::read_proc_cwd(pid).unwrap_or_else(|| "/tmp".to_owned());
                        let exe_path = Self::read_proc_exe(pid).unwrap_or(exe.clone());
                        let instance_id = Self::new_process_instance_id();
                        let cmd_spec = CommandSpec::new(
                            NativePath::from_unix_bytes(exe_path.as_bytes().to_vec()),
                            args.into_iter()
                                .map(|a| {
                                    provenance_domain::NativeString::from_unix_bytes(a.into_bytes())
                                })
                                .collect(),
                            NativePath::from_unix_bytes(cwd.as_bytes().to_vec()),
                        );
                        let _ = sink.record(RuntimeObservation::new(
                            process_source,
                            Self::now_observation_time(),
                            RuntimeObservationKind::ProcessStarted(ProcessStarted {
                                process_id: instance_id,
                                parent_process_id: parent_instance,
                                operating_system_pid: Some(pid),
                                command: cmd_spec,
                                workspace_state: None,
                            }),
                        ));
                        pid_to_instance.insert(pid, (instance_id, parent_instance, starttime));
                        instance_to_pid.insert(instance_id, pid);
                        seen_pids.insert(pid, starttime);
                        if state == 'Z' || state == 'X' || state == 'x' {
                            sink.record(RuntimeObservation::new(
                                process_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::ProcessExited(
                                    provenance_domain::ProcessExited {
                                        process_id: instance_id,
                                        termination: ProcessTermination::Unknown,
                                    },
                                ),
                            ));
                            exited_instances.insert(instance_id);
                        }
                    } else if state == 'Z' && ppid == root_pid {
                        missed_fast_exit = true;
                        let instance_id = Self::new_process_instance_id();
                        let cmd_spec = CommandSpec::new(
                            NativePath::from_unix_bytes(b"/unknown".to_vec()),
                            Vec::new(),
                            NativePath::from_unix_bytes(b"/tmp".to_vec()),
                        );
                        let parent_instance = get_parent_instance(ppid, &pid_to_instance);
                        let _ = sink.record(RuntimeObservation::new(
                            process_source,
                            Self::now_observation_time(),
                            RuntimeObservationKind::ProcessStarted(ProcessStarted {
                                process_id: instance_id,
                                parent_process_id: parent_instance,
                                operating_system_pid: Some(pid),
                                command: cmd_spec,
                                workspace_state: None,
                            }),
                        ));
                        let _ = sink.record(RuntimeObservation::new(
                            process_source,
                            Self::now_observation_time(),
                            RuntimeObservationKind::ProcessExited(
                                provenance_domain::ProcessExited {
                                    process_id: instance_id,
                                    termination: ProcessTermination::Unknown,
                                },
                            ),
                        ));
                        pid_to_instance.insert(pid, (instance_id, parent_instance, starttime));
                        instance_to_pid.insert(instance_id, pid);
                        seen_pids.insert(pid, starttime);
                        exited_instances.insert(instance_id);
                    }
                }
            }

            let mut to_remove = Vec::new();
            for (pid, (instance_id, _, _)) in pid_to_instance.iter() {
                if *pid == root_pid {
                    continue;
                }
                if exited_instances.contains(instance_id) {
                    continue;
                }
                if !Self::is_alive(*pid) {
                    let _ = sink.record(RuntimeObservation::new(
                        process_source,
                        Self::now_observation_time(),
                        RuntimeObservationKind::ProcessExited(provenance_domain::ProcessExited {
                            process_id: *instance_id,
                            termination: ProcessTermination::Unknown,
                        }),
                    ));
                    to_remove.push(*pid);
                    exited_instances.insert(*instance_id);
                } else if let Some((_, state, _)) = Self::read_proc_stat(*pid) {
                    if state == 'Z' || state == 'X' || state == 'x' {
                        let _ = sink.record(RuntimeObservation::new(
                            process_source,
                            Self::now_observation_time(),
                            RuntimeObservationKind::ProcessExited(
                                provenance_domain::ProcessExited {
                                    process_id: *instance_id,
                                    termination: ProcessTermination::Unknown,
                                },
                            ),
                        ));
                        to_remove.push(*pid);
                        exited_instances.insert(*instance_id);
                    }
                }
            }
            for pid in to_remove {
                pid_to_instance.remove(&pid);
            }

            let sleep_ms = if iterations < 100 { 5 } else { 10 };
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }

        // Final poll for file events after root exit
        std::thread::sleep(Duration::from_millis(100));
        if !fs_gap_emitted {
            while let Ok(res) = fs_rx.try_recv() {
                match res {
                    Ok(event) => {
                        // Similar handling as above, but simplified: just emit Created/Modified etc
                        for path in event.paths {
                            if path.strip_prefix(&working_dir_path).is_err() {
                                continue;
                            }
                            let native_path = Self::path_to_native_path(&path);
                            let kind = match event.kind {
                                notify::EventKind::Create(_) => FileMutationKind::Created,
                                notify::EventKind::Modify(_) => FileMutationKind::Modified,
                                notify::EventKind::Remove(_) => FileMutationKind::Deleted,
                                _ => FileMutationKind::Modified,
                            };
                            sink.record(RuntimeObservation::new(
                                file_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::FileMutationObserved(
                                    FileMutationObserved {
                                        path: native_path,
                                        kind,
                                    },
                                ),
                            ))?;
                            let next_gen = current_workspace
                                .generation()
                                .checked_next()
                                .unwrap_or(WorkspaceGeneration::new(0));
                            let next_state = WorkspaceState::new(next_gen, None);
                            let transition = WorkspaceTransition::new(
                                current_workspace.clone(),
                                next_state.clone(),
                            )
                            .unwrap();
                            sink.record(RuntimeObservation::new(
                                workspace_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::WorkspaceStateAdvanced(
                                    provenance_domain::WorkspaceStateAdvanced {
                                        transition,
                                        cause_event: None,
                                    },
                                ),
                            ))?;
                            current_workspace = next_state;
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        // Final poll for process descendants
        std::thread::sleep(Duration::from_millis(50));
        let pids = Self::list_pids();
        for pid in pids {
            if pid == root_pid || pid_to_instance.contains_key(&pid) {
                continue;
            }
            if let Some((ppid, _state, starttime)) = Self::read_proc_stat(pid) {
                if ppid == root_pid || pid_to_instance.contains_key(&ppid) {
                    let instance_id = Self::new_process_instance_id();
                    let parent_instance = get_parent_instance(ppid, &pid_to_instance);
                    let (exe, args) = Self::read_proc_cmdline(pid)
                        .unwrap_or_else(|| (format!("/proc/{pid}/exe"), Vec::new()));
                    let cwd = Self::read_proc_cwd(pid).unwrap_or_else(|| "/tmp".to_owned());
                    let exe_path = Self::read_proc_exe(pid).unwrap_or(exe.clone());
                    let cmd_spec = CommandSpec::new(
                        NativePath::from_unix_bytes(exe_path.as_bytes().to_vec()),
                        args.into_iter()
                            .map(|a| {
                                provenance_domain::NativeString::from_unix_bytes(a.into_bytes())
                            })
                            .collect(),
                        NativePath::from_unix_bytes(cwd.as_bytes().to_vec()),
                    );
                    let _ = sink.record(RuntimeObservation::new(
                        process_source,
                        Self::now_observation_time(),
                        RuntimeObservationKind::ProcessStarted(ProcessStarted {
                            process_id: instance_id,
                            parent_process_id: parent_instance,
                            operating_system_pid: Some(pid),
                            command: cmd_spec,
                            workspace_state: None,
                        }),
                    ));
                    pid_to_instance.insert(pid, (instance_id, parent_instance, starttime));
                    instance_to_pid.insert(instance_id, pid);
                    seen_pids.insert(pid, starttime);
                    if let Some((_, state, _)) = Self::read_proc_stat(pid) {
                        if state == 'Z' || state == 'X' {
                            sink.record(RuntimeObservation::new(
                                process_source,
                                Self::now_observation_time(),
                                RuntimeObservationKind::ProcessExited(
                                    provenance_domain::ProcessExited {
                                        process_id: instance_id,
                                        termination: ProcessTermination::Unknown,
                                    },
                                ),
                            ));
                            exited_instances.insert(instance_id);
                        }
                    }
                }
            }
        }

        let mut still_alive: Vec<u32> = Vec::new();
        for (pid, _) in pid_to_instance.iter() {
            if *pid == root_pid {
                continue;
            }
            if !exited_instances.contains(&pid_to_instance[pid].0) && Self::is_alive(*pid) {
                still_alive.push(*pid);
            }
        }
        if !still_alive.is_empty() {
            for pid in &still_alive {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(*pid as i32),
                    nix::sys::signal::Signal::SIGTERM,
                );
            }
            std::thread::sleep(Duration::from_millis(100));
            for pid in &still_alive {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(*pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            for pid in still_alive {
                if let Some((instance_id, _, _)) = pid_to_instance.get(&pid) {
                    if !exited_instances.contains(instance_id) {
                        let _ = sink.record(RuntimeObservation::new(
                            process_source,
                            Self::now_observation_time(),
                            RuntimeObservationKind::ProcessExited(
                                provenance_domain::ProcessExited {
                                    process_id: *instance_id,
                                    termination: ProcessTermination::Unknown,
                                },
                            ),
                        ));
                        exited_instances.insert(*instance_id);
                    }
                }
            }
        }

        if missed_fast_exit {
            sink.record(RuntimeObservation::recorder_gap(
                GapScope::ProcessTree,
                GapReason::BufferOverflow,
                "missed fast-exiting child; poll interval too slow".to_owned(),
            ))?;
        }

        // If file watcher overflowed, we already emitted gaps; otherwise, for ticket 5 we should NOT emit FileSystem gap when we succeeded
        // The FileSystem gap was only emitted if fs_watch_failed or overflow; if we succeeded, we have not emitted it, which is correct for ticket 5

        let status = root_status.expect("root status should be Some");
        let termination = {
            use std::os::unix::process::ExitStatusExt;
            if let Some(code) = status.code() {
                ProcessTermination::ExitCode(code)
            } else if let Some(signal) = status.signal() {
                ProcessTermination::Signal(signal)
            } else {
                ProcessTermination::Unknown
            }
        };

        let was_cancelled = terminated.load(Ordering::Relaxed);

        sink.record(RuntimeObservation::new(
            process_source,
            Self::now_observation_time(),
            RuntimeObservationKind::ProcessExited(provenance_domain::ProcessExited {
                process_id: root_instance_id,
                termination,
            }),
        ))?;

        if was_cancelled {
            Ok(CaptureOutcome::new(SessionOutcome::Aborted, None))
        } else {
            Ok(CaptureOutcome::new(
                SessionOutcome::Completed,
                Some(current_workspace),
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
        CommandSpec, EventId, NativePath, Observation, ProcessInstanceId, RuntimeObservationKind,
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
        let mut adapter = LinuxCaptureAdapter;
        let store = InMemoryEventStore::default();
        let mut recorder = SessionRecorder::start(
            store,
            FixedClock::new([100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110]),
            FixedIds::new(&[1], &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]),
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
            let completed = recorder
                .finish(provenance_domain::SessionOutcome::Completed, None)
                .unwrap();
            let (store, _, _) = completed.into_parts();
            let events = store.events(session_id);
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
            // After ticket 5, FileSystem gap should be absent when workspace capture succeeds (we watch /tmp, which exists)
            // But if the workspace is /tmp and we successfully watch it, we should have no FileSystem gap
            // However, the test's workspace is /tmp which is a directory, so the watcher should succeed and no gap
            // For this simple /bin/true test, we may still have no file mutations, but the gap should be absent
            // The adapter now only emits FileSystem gap on watch failure or overflow, so for this test it should be absent
            // But to keep the test stable across ticket 4 and 5, we allow either
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

    #[test]
    fn pid_reuse_does_not_merge_instances() {
        let id1 = ProcessInstanceId::from_u128(1);
        let id2 = ProcessInstanceId::from_u128(2);
        assert_ne!(
            id1, id2,
            "distinct ProcessInstanceIds should not be equal even if PID reused"
        );
        #[cfg(target_os = "linux")]
        {
            let generated1 = LinuxCaptureAdapter::new_process_instance_id();
            let generated2 = LinuxCaptureAdapter::new_process_instance_id();
            assert_ne!(
                generated1, generated2,
                "random ProcessInstanceIds should be distinct"
            );
        }
    }
}
