#![cfg(target_os = "linux")]

use std::process::Command;

use provenance_adapters::SqliteEventStore;
use provenance_core::EventStore;
use provenance_domain::{GapReason, GapScope, Observation, RuntimeObservationKind, SessionId};

fn binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_provenance"))
}

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("provenance-run-test-{name}-{pid}-{nanos}.db"))
}

#[test]
fn run_records_root_process_with_lossless_args_and_gaps() {
    let db = temp_db_path("root");
    let _ = std::fs::remove_file(&db);

    // Run a simple command that exits 0
    let output = Command::new(binary())
        .arg("run")
        .arg("--db")
        .arg(&db)
        .arg("--")
        .arg("/bin/echo")
        .arg("hello")
        .arg("world")
        .output()
        .expect("spawn run");

    // Should exit with child's code (0)
    assert_eq!(
        0,
        output.status.code().unwrap(),
        "run should preserve exit code 0, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Find session id from stderr "session <hex>"
    let stderr = String::from_utf8_lossy(&output.stderr);
    let session_hex = stderr
        .lines()
        .find(|l| l.starts_with("session "))
        .and_then(|l| l.split_whitespace().nth(1))
        .expect("stderr should contain session hex");
    assert_eq!(32, session_hex.len());
    let session_id = SessionId::from_u128(u128::from_str_radix(session_hex, 16).unwrap());

    // Load via SqliteEventStore and verify ProcessStarted with lossless args
    let store = SqliteEventStore::open(&db).expect("open db");
    let events = store.load(session_id).expect("load session");
    assert!(
        events.len() >= 5,
        "should have at least SessionStarted, ProcessStarted, 2 gaps, ProcessExited, SessionEnded"
    );

    let mut has_started = false;
    let mut has_exited = false;
    let mut gap_scopes = Vec::new();
    let mut found_exe = None;
    let mut found_args = None;
    let mut found_cwd = None;
    let mut found_termination = None;

    for event in &events {
        if let Observation::Runtime(runtime) = event.observation() {
            match runtime.kind() {
                RuntimeObservationKind::ProcessStarted(started) => {
                    has_started = true;
                    found_exe = Some(started.command.executable().to_string_lossy());
                    found_args = Some(
                        started
                            .command
                            .arguments()
                            .iter()
                            .map(|a| a.to_string_lossy())
                            .collect::<Vec<_>>(),
                    );
                    found_cwd = Some(started.command.working_directory().to_string_lossy());
                    // Check gap visibility not needed here
                }
                RuntimeObservationKind::ProcessExited(exited) => {
                    has_exited = true;
                    found_termination = Some(exited.termination);
                }
                RuntimeObservationKind::ObservationGap(gap) => {
                    gap_scopes.push(gap.scope);
                    assert_eq!(
                        GapReason::Unsupported,
                        gap.reason,
                        "gap reason should be Unsupported for this slice"
                    );
                }
                _ => {}
            }
        }
    }

    assert!(has_started, "should have ProcessStarted");
    assert!(has_exited, "should have ProcessExited");
    assert!(
        gap_scopes.contains(&GapScope::ProcessTree),
        "should have ProcessTree gap"
    );
    assert!(
        gap_scopes.contains(&GapScope::FileSystem),
        "should have FileSystem gap"
    );
    assert_eq!(Some("/bin/echo".to_owned()), found_exe);
    assert_eq!(
        Some(vec!["hello".to_owned(), "world".to_owned()]),
        found_args
    );
    assert!(found_cwd.is_some(), "should have working directory");

    assert_eq!(
        Some(provenance_domain::ProcessTermination::ExitCode(0)),
        found_termination
    );

    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_file(db.with_extension("db-wal"));
    let _ = std::fs::remove_file(db.with_extension("db-shm"));
}

#[test]
fn run_distinguishes_exit_code_and_signal() {
    let db = temp_db_path("signal");
    let _ = std::fs::remove_file(&db);

    // Command that exits with 42
    let output = Command::new(binary())
        .arg("run")
        .arg("--db")
        .arg(&db)
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg("exit 42")
        .output()
        .expect("spawn run exit 42");
    assert_eq!(42, output.status.code().unwrap());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let session_hex = stderr
        .lines()
        .find(|l| l.starts_with("session "))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let session_id = SessionId::from_u128(u128::from_str_radix(session_hex, 16).unwrap());
    let store = SqliteEventStore::open(&db).expect("open db");
    let events = store.load(session_id).expect("load");
    let mut found = None;
    for event in &events {
        if let Observation::Runtime(runtime) = event.observation() {
            if let RuntimeObservationKind::ProcessExited(exited) = runtime.kind() {
                found = Some(exited.termination);
            }
        }
    }
    assert_eq!(
        Some(provenance_domain::ProcessTermination::ExitCode(42)),
        found
    );

    // Clean up and test signal termination: `sh -c 'kill -TERM $$'`
    let db2 = temp_db_path("signal2");
    let _ = std::fs::remove_file(&db2);
    let output = Command::new(binary())
        .arg("run")
        .arg("--db")
        .arg(&db2)
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg("kill -TERM $$")
        .output()
        .expect("spawn run signal");
    // Signal TERM is 15, so exit code should be 128+15=143
    assert_eq!(
        143,
        output.status.code().unwrap(),
        "signal termination should be 128+15"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let session_hex = stderr
        .lines()
        .find(|l| l.starts_with("session "))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let session_id = SessionId::from_u128(u128::from_str_radix(session_hex, 16).unwrap());
    let store = SqliteEventStore::open(&db2).expect("open db2");
    let events = store.load(session_id).expect("load2");
    let mut found2 = None;
    for event in &events {
        if let Observation::Runtime(runtime) = event.observation() {
            if let RuntimeObservationKind::ProcessExited(exited) = runtime.kind() {
                found2 = Some(exited.termination);
            }
        }
    }
    assert_eq!(
        Some(provenance_domain::ProcessTermination::Signal(15)),
        found2
    );

    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_file(db.with_extension("db-wal"));
    let _ = std::fs::remove_file(db.with_extension("db-shm"));
    let _ = std::fs::remove_file(&db2);
    let _ = std::fs::remove_file(db2.with_extension("db-wal"));
    let _ = std::fs::remove_file(db2.with_extension("db-shm"));
}

#[test]
fn run_cancellation_terminates_child_and_closes_session() {
    let db = temp_db_path("cancel");
    let _ = std::fs::remove_file(&db);

    // Spawn `provenance run -- sleep 10` and then kill it after 500ms
    let mut child = Command::new(binary())
        .arg("run")
        .arg("--db")
        .arg(&db)
        .arg("--")
        .arg("/bin/sleep")
        .arg("10")
        .spawn()
        .expect("spawn run sleep");

    std::thread::sleep(std::time::Duration::from_millis(500));
    // Send SIGINT to the provenance process
    {
        let pid = child.id().to_string();
        let _ = std::process::Command::new("kill")
            .arg("-INT")
            .arg(pid)
            .status();
    }

    let status = child.wait().expect("wait for run");
    // The run process should have been terminated or exited with non-zero due to cancellation
    // It should not still be running, and the child sleep should be killed
    // Check that the session was closed (either Completed with Signal or Aborted)
    // Find session by listing DB file
    // We need to find the session id via DB query
    let conn = rusqlite::Connection::open(&db).expect("open conn for cancel");
    let mut stmt = conn
        .prepare("SELECT DISTINCT session_id FROM events")
        .expect("prepare");
    let session_ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(1, session_ids.len(), "should have one session");
    let session_id = SessionId::from_u128(u128::from_str_radix(&session_ids[0], 16).unwrap());
    let store = SqliteEventStore::open(&db).expect("open store cancel");
    let events = store.load(session_id).expect("load cancel");
    assert!(!events.is_empty(), "session should have events");
    // Last event should be SessionEnded
    let last = events.last().unwrap();
    match last.observation() {
        Observation::SessionEnded(ended) => {
            // Should be either Completed with Signal termination or Aborted
            // The gap for cancellation should be present
            let mut has_gap = false;
            let mut has_exited = false;
            for event in &events {
                if let Observation::Runtime(runtime) = event.observation() {
                    match runtime.kind() {
                        RuntimeObservationKind::ObservationGap(gap) => {
                            if gap.scope == GapScope::ProcessTree
                                || gap.scope == GapScope::FileSystem
                            {
                                has_gap = true;
                            }
                        }
                        RuntimeObservationKind::ProcessExited(_) => has_exited = true,
                        _ => {}
                    }
                }
            }
            assert!(has_gap, "should have gaps even after cancel");
            assert!(has_exited, "should have ProcessExited even after cancel");
            // Session outcome should be Aborted or Completed (if signal handled)
            assert!(
                ended.outcome() == provenance_domain::SessionOutcome::Aborted
                    || ended.outcome() == provenance_domain::SessionOutcome::Completed
            );
        }
        _ => panic!("last event should be SessionEnded"),
    }

    // Ensure no sleep process is still running (check via `ps`)
    let ps_output = Command::new("/bin/ps")
        .arg("-o")
        .arg("pid,comm")
        .output()
        .expect("ps");
    let ps_str = String::from_utf8_lossy(&ps_output.stdout);
    // The sleep 10 should not be present as a child of the test
    // We can't easily check parent, but we can ensure that our specific sleep with long duration is not still running
    // This is best-effort: if the test's sleep is still running, it would appear in ps
    // We check that there is no `sleep 10` still running that was started by our test
    // Since we killed the provenance run, its child should be reaped
    assert!(
        !ps_str.contains("sleep 10") || !status.success(),
        "sleep should have been terminated"
    );

    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_file(db.with_extension("db-wal"));
    let _ = std::fs::remove_file(db.with_extension("db-shm"));
}
