#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use provenance_core::{EventStore, EventStoreError};
use provenance_domain::EVENT_SCHEMA_VERSION;

mod projections;
mod timeline;

use projections::{
    format_changes_human, format_changes_json, format_processes_human, format_processes_json,
    format_state_human, format_state_json,
};
use timeline::{format_human, format_json, load_events, parse_session_id, resolve_db_path};

fn main() -> ExitCode {
    ExitCode::from(run(std::env::args_os().skip(1)))
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let mut arguments: Vec<OsString> = arguments.into_iter().collect();
    if arguments.is_empty() {
        print_help();
        return 0;
    }

    let command = arguments.remove(0);
    let Some(command_str) = command.to_str() else {
        eprintln!("command is not valid UTF-8");
        return 2;
    };

    match command_str {
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        "version" | "--version" | "-V" => {
            println!("provenance {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "schema-version" => {
            println!("{EVENT_SCHEMA_VERSION}");
            0
        }
        "timeline" => run_timeline(&arguments),
        "processes" => run_processes(&arguments),
        "changes" => run_changes(&arguments),
        "state" => run_state(&arguments),
        "run" => run_run(&arguments),
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("run `provenance --help` for usage");
            2
        }
    }
}

fn run_timeline(arguments: &[OsString]) -> u8 {
    // Parse: timeline <SESSION_ID> [--db <PATH>] [--format human|json] [--json]
    let mut session_id_raw: Option<OsString> = None;
    let mut db_path: Option<PathBuf> = None;
    let mut format = String::from("human");

    let mut iter = arguments.iter();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            eprintln!("argument is not valid UTF-8");
            return 2;
        };
        match text {
            "--db" | "--database" => {
                let Some(value) = iter.next() else {
                    eprintln!("--db requires a value");
                    return 2;
                };
                let Some(value_str) = value.to_str() else {
                    eprintln!("--db value is not valid UTF-8");
                    return 2;
                };
                db_path = Some(PathBuf::from(value_str));
            }
            "--format" => {
                let Some(value) = iter.next() else {
                    eprintln!("--format requires a value (human|json)");
                    return 2;
                };
                let Some(value_str) = value.to_str() else {
                    eprintln!("--format value is not valid UTF-8");
                    return 2;
                };
                if value_str != "human" && value_str != "json" {
                    eprintln!("--format must be human or json");
                    return 2;
                }
                format = value_str.to_owned();
            }
            "--json" => {
                format = "json".to_owned();
            }
            "--help" | "-h" => {
                print_timeline_help();
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("unknown option for timeline: {other}");
                eprintln!("run `provenance timeline --help` for usage");
                return 2;
            }
            _ => {
                if session_id_raw.is_none() {
                    session_id_raw = Some(arg.clone());
                } else {
                    eprintln!("unexpected argument: {text}");
                    eprintln!("run `provenance timeline --help` for usage");
                    return 2;
                }
            }
        }
    }

    let Some(session_raw) = session_id_raw else {
        eprintln!("timeline requires a session id");
        eprintln!("run `provenance timeline --help` for usage");
        return 2;
    };
    let Some(session_str) = session_raw.to_str() else {
        eprintln!("session id is not valid UTF-8");
        return 2;
    };

    let session_id = match parse_session_id(session_str) {
        Ok(id) => id,
        Err(detail) => {
            eprintln!("invalid session id: {detail}");
            return 2;
        }
    };

    let db_path = resolve_db_path(db_path.as_deref());

    let events = match load_events(&db_path, session_id) {
        Ok(events) => events,
        Err(EventStoreError::Corrupt(detail)) => {
            eprintln!("corrupt row for session {}: {detail}", session_str);
            return 4;
        }
        Err(EventStoreError::Unavailable(detail)) => {
            if detail.to_lowercase().contains("corrupt") {
                eprintln!("corrupt row for session {}: {detail}", session_str);
                return 4;
            }
            eprintln!("event store unavailable: {detail}");
            return 5;
        }
        Err(other) => {
            eprintln!("failed to load session {}: {other}", session_str);
            return 5;
        }
    };

    if events.is_empty() {
        eprintln!("unknown session: {session_str}");
        return 3;
    }

    if format == "json" {
        match format_json(session_id, &events) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("failed to render json: {error}");
                5
            }
        }
    } else {
        let human = format_human(session_id, &events);
        print!("{human}");
        0
    }
}

fn run_processes(arguments: &[OsString]) -> u8 {
    run_projection(
        arguments,
        "processes",
        format_processes_human,
        format_processes_json,
    )
}

fn run_changes(arguments: &[OsString]) -> u8 {
    run_projection(
        arguments,
        "changes",
        format_changes_human,
        format_changes_json,
    )
}

fn run_state(arguments: &[OsString]) -> u8 {
    run_projection(arguments, "state", format_state_human, format_state_json)
}

fn run_projection(
    arguments: &[OsString],
    name: &str,
    format_human_fn: fn(
        provenance_domain::SessionId,
        &[provenance_domain::EventEnvelope],
    ) -> String,
    format_json_fn: fn(
        provenance_domain::SessionId,
        &[provenance_domain::EventEnvelope],
    ) -> Result<String, String>,
) -> u8 {
    let mut session_id_raw: Option<OsString> = None;
    let mut db_path: Option<std::path::PathBuf> = None;
    let mut format = String::from("human");

    let mut iter = arguments.iter();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            eprintln!("argument is not valid UTF-8");
            return 2;
        };
        match text {
            "--db" | "--database" => {
                let Some(value) = iter.next() else {
                    eprintln!("--db requires a value");
                    return 2;
                };
                let Some(value_str) = value.to_str() else {
                    eprintln!("--db value is not valid UTF-8");
                    return 2;
                };
                db_path = Some(std::path::PathBuf::from(value_str));
            }
            "--format" => {
                let Some(value) = iter.next() else {
                    eprintln!("--format requires a value (human|json)");
                    return 2;
                };
                let Some(value_str) = value.to_str() else {
                    eprintln!("--format value is not valid UTF-8");
                    return 2;
                };
                if value_str != "human" && value_str != "json" {
                    eprintln!("--format must be human or json");
                    return 2;
                }
                format = value_str.to_owned();
            }
            "--json" => {
                format = "json".to_owned();
            }
            "--help" | "-h" => {
                eprintln!(
                    "usage: provenance {name} <SESSION_ID> [--db <PATH>] [--format human|json]"
                );
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("unknown option for {name}: {other}");
                return 2;
            }
            _ => {
                if session_id_raw.is_none() {
                    session_id_raw = Some(arg.clone());
                } else {
                    eprintln!("unexpected argument: {text}");
                    return 2;
                }
            }
        }
    }

    let Some(session_raw) = session_id_raw else {
        eprintln!("{name} requires a session id");
        return 2;
    };
    let Some(session_str) = session_raw.to_str() else {
        eprintln!("session id is not valid UTF-8");
        return 2;
    };

    let session_id = match parse_session_id(session_str) {
        Ok(id) => id,
        Err(detail) => {
            eprintln!("invalid session id: {detail}");
            return 2;
        }
    };

    let db_path = resolve_db_path(db_path.as_deref());

    let events = match load_events(&db_path, session_id) {
        Ok(events) => events,
        Err(provenance_core::EventStoreError::Corrupt(detail)) => {
            eprintln!("corrupt row for session {}: {detail}", session_str);
            return 4;
        }
        Err(provenance_core::EventStoreError::Unavailable(detail)) => {
            if detail.to_lowercase().contains("corrupt") {
                eprintln!("corrupt row for session {}: {detail}", session_str);
                return 4;
            }
            eprintln!("event store unavailable: {detail}");
            return 5;
        }
        Err(other) => {
            eprintln!("failed to load session {}: {other}", session_str);
            return 5;
        }
    };

    if events.is_empty() {
        eprintln!("unknown session: {session_str}");
        return 3;
    }

    if format == "json" {
        match format_json_fn(session_id, &events) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("failed to render json: {error}");
                5
            }
        }
    } else {
        let human = format_human_fn(session_id, &events);
        print!("{human}");
        0
    }
}

fn run_run(arguments: &[OsString]) -> u8 {
    // Parse: run [--db <PATH>] -- <executable> [args...]
    let mut db_path: Option<PathBuf> = None;
    let mut executable: Option<OsString> = None;
    let mut exec_args: Vec<OsString> = Vec::new();
    let mut seen_separator = false;

    let mut iter = arguments.iter().peekable();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            eprintln!("argument is not valid UTF-8");
            return 2;
        };
        if !seen_separator {
            match text {
                "--db" | "--database" => {
                    let Some(value) = iter.next() else {
                        eprintln!("--db requires a value");
                        return 2;
                    };
                    let Some(value_str) = value.to_str() else {
                        eprintln!("--db value is not valid UTF-8");
                        return 2;
                    };
                    db_path = Some(PathBuf::from(value_str));
                }
                "--help" | "-h" => {
                    print_run_help();
                    return 0;
                }
                "--" => {
                    seen_separator = true;
                }
                other if other.starts_with('-') => {
                    eprintln!("unknown option for run: {other}");
                    eprintln!("run `provenance run --help` for usage");
                    return 2;
                }
                _ => {
                    // Implicit separator if no -- but we have executable
                    // Treat first non-option as executable and the rest as args
                    // But spec requires --, so we treat this as missing separator
                    eprintln!("run requires -- separator before command");
                    eprintln!("run `provenance run --help` for usage");
                    return 2;
                }
            }
        } else {
            if executable.is_none() {
                executable = Some(arg.clone());
            } else {
                exec_args.push(arg.clone());
            }
        }
    }

    if !seen_separator {
        eprintln!("run requires -- separator");
        eprintln!("run `provenance run --help` for usage");
        return 2;
    }

    let Some(exe) = executable else {
        eprintln!("run requires a command after --");
        eprintln!("run `provenance run --help` for usage");
        return 2;
    };

    let db_path = resolve_db_path(db_path.as_deref());

    // Build CommandSpec with lossless native paths
    let exe_native = os_string_to_native_path(&exe);
    let args_native: Vec<provenance_domain::NativeString> =
        exec_args.iter().map(os_string_to_native_string).collect();
    let cwd_native = {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStrExt;
            provenance_domain::NativePath::from_unix_bytes(cwd.as_os_str().as_bytes().to_vec())
        }
        #[cfg(not(target_os = "linux"))]
        {
            provenance_domain::NativePath::from_unix_bytes(
                cwd.to_string_lossy().as_bytes().to_vec(),
            )
        }
    };

    let command_spec = provenance_domain::CommandSpec::new(exe_native, args_native, cwd_native);

    // Prepare store, clock, ids, and capture adapter
    let store = match provenance_adapters::SqliteEventStore::open(&db_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "failed to open event store at {}: {error}",
                db_path.display()
            );
            return 5;
        }
    };

    let clock = provenance_adapters::SystemClock;
    let ids = provenance_adapters::RandomIdGenerator;
    let mut capture = provenance_adapters::platform::linux::LinuxCaptureAdapter;

    let request = provenance_core::CaptureRequest::new(
        command_spec,
        Some(provenance_domain::WorkspaceState::initial()),
    );

    // Record execution (this will handle gaps and session lifecycle)
    let execution =
        match provenance_core::record_execution(store, clock, ids, &mut capture, request) {
            Ok(exec) => exec,
            Err(error) => {
                eprintln!("failed to record execution: {error}");
                return 5;
            }
        };

    // Determine wrapped exit code from the recorded ProcessExited
    let session_id = execution.session().session_id();
    let (store_after, _, _) = execution.into_session().into_parts();
    // Load events to find ProcessExited termination for exit code preservation
    let events = match store_after.load(session_id) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("failed to load session for exit code: {error}");
            return 5;
        }
    };

    // Find the ProcessExited for the root process
    let mut exit_code: u8 = 0;
    let mut found_termination = None;
    for event in &events {
        if let provenance_domain::Observation::Runtime(runtime) = event.observation() {
            if let provenance_domain::RuntimeObservationKind::ProcessExited(exited) = runtime.kind()
            {
                found_termination = Some(exited.termination);
            }
        }
    }

    if let Some(termination) = found_termination {
        match termination {
            provenance_domain::ProcessTermination::ExitCode(code) => {
                // Clamp to u8 for exit code (Unix exit codes are 0-255)
                exit_code = u8::try_from(code).unwrap_or(1);
                // Also handle negative codes?
                if code < 0 {
                    exit_code = 1;
                }
            }
            provenance_domain::ProcessTermination::Signal(signal) => {
                // Conventional Unix: 128 + signal
                exit_code = u8::try_from(128 + signal).unwrap_or(1);
            }
            provenance_domain::ProcessTermination::Unknown => {
                exit_code = 1;
            }
        }
    }

    // Optionally print session id for the user to later query
    eprintln!("session {:032x}", session_id.as_u128());

    exit_code
}

#[cfg(target_os = "linux")]
fn os_string_to_native_string(value: &OsString) -> provenance_domain::NativeString {
    use std::os::unix::ffi::OsStrExt;
    provenance_domain::NativeString::from_unix_bytes(value.as_bytes().to_vec())
}

#[cfg(not(target_os = "linux"))]
fn os_string_to_native_string(value: &OsString) -> provenance_domain::NativeString {
    provenance_domain::NativeString::from_unix_bytes(value.to_string_lossy().as_bytes().to_vec())
}

#[cfg(target_os = "linux")]
fn os_string_to_native_path(value: &OsString) -> provenance_domain::NativePath {
    use std::os::unix::ffi::OsStrExt;
    provenance_domain::NativePath::from_unix_bytes(value.as_bytes().to_vec())
}

#[cfg(not(target_os = "linux"))]
fn os_string_to_native_path(value: &OsString) -> provenance_domain::NativePath {
    provenance_domain::NativePath::from_unix_bytes(value.to_string_lossy().as_bytes().to_vec())
}

fn print_help() {
    println!(
        "provenance {version}\n\nUSAGE:\n    provenance <COMMAND>\n\nCOMMANDS:\n    run             Execute a command and record its process lifecycle\n    timeline        Show the ordered event stream for a session\n    processes       Show process-tree projection for a session\n    changes         Show file-mutation projection for a session\n    state           Show workspace-state projection for a session\n    schema-version  Print the raw-event schema version\n    version         Print the binary version\n    help            Print this help\n\nRun `provenance <COMMAND> --help` for more information.",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn print_timeline_help() {
    println!(
        "provenance timeline {version}\n\nUSAGE:\n    provenance timeline <SESSION_ID> [OPTIONS]\n\nARGS:\n    <SESSION_ID>  32-hex session id (e.g., 00000000000000000000000000000001)\n\nOPTIONS:\n    --db <PATH>          SQLite database path (default: .provenance/provenance.db or $PROVENANCE_DB)\n    --format <human|json>  Output format (default: human)\n    --json               Shorthand for --format json\n    -h, --help           Print this help\n",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn print_run_help() {
    println!(
        "provenance run {version}\n\nUSAGE:\n    provenance run [OPTIONS] -- <COMMAND> [ARGS...]\n\nOPTIONS:\n    --db <PATH>  SQLite database path (default: .provenance/provenance.db or $PROVENANCE_DB)\n    -h, --help   Print this help\n\nARGS:\n    <COMMAND>  Executable to run (lossless native path)\n    [ARGS...]  Arguments to the command (lossless)\n\nThe run adapter records the root process start/exit and emits explicit gaps for\nunsupported descendant-process and filesystem coverage until later slices replace them.\n",
        version = env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::run;

    #[test]
    fn unknown_command_returns_usage_error() {
        assert_eq!(2, run([OsString::from("unknown")]));
    }

    #[test]
    fn help_returns_success() {
        assert_eq!(0, run([OsString::from("--help")]));
    }

    #[test]
    fn timeline_requires_session_id() {
        assert_eq!(2, run([OsString::from("timeline")]));
    }

    #[test]
    fn timeline_rejects_invalid_session_id() {
        assert_eq!(
            2,
            run([OsString::from("timeline"), OsString::from("nothex")])
        );
    }

    #[test]
    fn run_requires_separator() {
        assert_eq!(2, run([OsString::from("run"), OsString::from("/bin/echo")]));
    }

    #[test]
    fn run_requires_command_after_separator() {
        assert_eq!(2, run([OsString::from("run"), OsString::from("--")]));
    }
}
