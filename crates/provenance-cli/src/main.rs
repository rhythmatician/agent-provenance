#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::process::ExitCode;

use provenance_domain::EVENT_SCHEMA_VERSION;

fn main() -> ExitCode {
    ExitCode::from(run(std::env::args_os().skip(1)))
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        print_help();
        return 0;
    };

    match command.to_str() {
        Some("help" | "--help" | "-h") => {
            print_help();
            0
        }
        Some("version" | "--version" | "-V") => {
            println!("provenance {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("schema-version") => {
            println!("{EVENT_SCHEMA_VERSION}");
            0
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!("run `provenance --help` for usage");
            2
        }
        None => {
            eprintln!("command is not valid UTF-8");
            2
        }
    }
}

fn print_help() {
    println!(
        "provenance {version}\n\nUSAGE:\n    provenance <COMMAND>\n\nCOMMANDS:\n    schema-version  Print the raw-event schema version\n    version         Print the binary version\n    help            Print this help\n\nThe command-capture and durable-store adapters are intentionally not implemented in the bootstrap scaffold.",
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
}
