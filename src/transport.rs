//! TCP transport via Apple-signed /usr/bin/nc. Not an in-process relay:
//! macOS Local Network privacy silently blackholes connects from unsigned
//! binaries in contexts without a grant, and a nix-built binary's TCC
//! identity changes on every rebuild, so it can never hold one. -G bounds
//! the connect time so a stale IP fails fast instead of hitting the ~75 s
//! OS default.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub fn reachable(ip: &str, port: u16) -> bool {
    Command::new("/usr/bin/nc")
        .args(["-z", "-G", "3", ip, &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Replace this process with the transport. Only returns on exec failure.
pub fn exec(ip: &str, port: u16) -> std::io::Error {
    Command::new("/usr/bin/nc")
        .args(["-G", "3", ip, &port.to_string()])
        .exec()
}
