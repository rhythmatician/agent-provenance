// This file is intentionally allowed to contain the single unsafe pre_exec for setsid.
// SAFETY: The function setsid_in_child is called only from Command::pre_exec, which is
// executed after fork and before exec in the child process. It must be async-signal-safe
// and must not allocate. It calls only nix::unistd::setsid, which is a direct syscall.
#![allow(unsafe_code)]

use std::os::unix::process::CommandExt;
use std::process::Command;

/// SAFETY: Must be called only as a pre_exec callback. See module SAFETY above.
pub fn setsid_in_child(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            // SAFETY: setsid is async-signal-safe; no allocation, no locking.
            nix::unistd::setsid()
                .map(|_| ())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))
        });
    }
}
