//! macOS listening sockets through `lsof`, limited to this user's processes.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{parse_lsof, SocketListener};

const LSOF_TIMEOUT: Duration = Duration::from_secs(5);

pub fn listeners() -> Result<Vec<SocketListener>, String> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let mut child = Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-u",
            &uid.to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
            "-FpPtn",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot run lsof: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > LSOF_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("lsof timed out".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(format!("lsof failed: {error}")),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("lsof failed: {error}"))?;
    // lsof exits 1 when nothing matched; its stdout is still authoritative.
    let mut rows = parse_lsof(&String::from_utf8_lossy(&output.stdout));
    rows.sort_by(|a, b| {
        (a.port, a.family as u8, &a.address).cmp(&(b.port, b.family as u8, &b.address))
    });
    Ok(rows)
}
