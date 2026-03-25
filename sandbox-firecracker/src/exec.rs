use anyhow::{bail, Result};
use kvm_ioctls::VcpuExit;
use std::time::{Duration, Instant};

use crate::kvm::serial::is_serial_port;
use crate::types::{ExecResult, VmInstance};

fn make_done_marker() -> String {
    format!("DONE_{:032x}", uuid::Uuid::new_v4().as_u128())
}

pub fn run_code(
    vm: &mut VmInstance,
    code: &str,
    language: &str,
    timeout: Duration,
    max_output: usize,
) -> Result<ExecResult> {
    let (ext, interpreter) = match language {
        "python" | "python3" => ("py", "python3"),
        "node" | "javascript" | "js" => ("js", "node"),
        "ruby" => ("rb", "ruby"),
        "bash" | "sh" => ("sh", "bash"),
        other => bail!("unsupported language: {other}"),
    };

    let delim = format!("HEREDOC_{:016x}", rand_u64());
    let marker = make_done_marker();

    let cmd = format!(
        "cat <<'{delim}' > /tmp/code.{ext}\n{code}\n{delim}\n{interpreter} /tmp/code.{ext}; EXIT_CODE=$?; echo \"${{EXIT_CODE}} {marker}\"\n"
    );

    run_command_with_marker(vm, &cmd, &marker, timeout, max_output)
}

pub fn run_command(
    vm: &mut VmInstance,
    command: &str,
    timeout: Duration,
    max_output: usize,
) -> Result<ExecResult> {
    let marker = make_done_marker();
    let wrapped = format!("{command}; EXIT_CODE=$?; echo \"${{EXIT_CODE}} {marker}\"\n");
    run_command_with_marker(vm, &wrapped, &marker, timeout, max_output)
}

fn run_command_with_marker(
    vm: &mut VmInstance,
    command: &str,
    marker: &str,
    timeout: Duration,
    max_output: usize,
) -> Result<ExecResult> {
    let start = Instant::now();
    let deadline = start + timeout;

    let input = format!("{command}\n");
    vm.serial.queue_input(input.as_bytes());

    loop {
        if Instant::now() > deadline {
            let raw = vm.serial.take_output();
            vm.serial.clear_output();
            let truncated = truncate_output(&raw, max_output);
            return Ok(ExecResult {
                exit_code: -1,
                stdout: truncated,
                stderr: "execution timed out".to_string(),
                duration_us: start.elapsed().as_micros() as u64,
            });
        }

        match vm.vcpu_fd.run() {
            Ok(VcpuExit::IoOut(port, data)) => {
                if is_serial_port(port) {
                    vm.serial.handle_io_out(port, data);

                    if vm.serial.output_contains(marker) {
                        let raw = vm.serial.take_output();
                        let (exit_code, stdout) = parse_exit_code(&raw, marker);
                        let truncated = truncate_output(&stdout, max_output);
                        return Ok(ExecResult {
                            exit_code,
                            stdout: truncated,
                            stderr: String::new(),
                            duration_us: start.elapsed().as_micros() as u64,
                        });
                    }
                }
            }
            Ok(VcpuExit::IoIn(port, data)) => {
                if is_serial_port(port) {
                    let byte = vm.serial.handle_io_in(port);
                    if !data.is_empty() {
                        data[0] = byte;
                    }
                }
            }
            Ok(VcpuExit::Hlt) => {
                let raw = vm.serial.take_output();
                let truncated = truncate_output(&raw, max_output);
                return Ok(ExecResult {
                    exit_code: -2,
                    stdout: truncated,
                    stderr: "VM halted".to_string(),
                    duration_us: start.elapsed().as_micros() as u64,
                });
            }
            Ok(VcpuExit::Shutdown) => {
                let raw = vm.serial.take_output();
                let truncated = truncate_output(&raw, max_output);
                return Ok(ExecResult {
                    exit_code: -3,
                    stdout: truncated,
                    stderr: "VM shutdown".to_string(),
                    duration_us: start.elapsed().as_micros() as u64,
                });
            }
            Ok(_) => {}
            Err(e) => {
                if e.errno() == libc::EAGAIN || e.errno() == libc::EINTR {
                    continue;
                }
                bail!("KVM_RUN failed: {e}");
            }
        }
    }
}

fn parse_exit_code(raw: &str, marker: &str) -> (i32, String) {
    if let Some(marker_pos) = raw.rfind(marker) {
        let before_marker = &raw[..marker_pos];

        let code_start = before_marker.rfind('\n').map(|p| p + 1).unwrap_or(0);

        let code_str = before_marker[code_start..].trim();
        let exit_code = code_str.parse::<i32>().unwrap_or(-1);
        let stdout = before_marker[..code_start].to_string();

        (exit_code, stdout)
    } else {
        (-1, raw.to_string())
    }
}

fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}... [truncated]", &s[..end])
    }
}

fn rand_u64() -> u64 {
    let mut buf = [0u8; 8];
    // SAFETY: getrandom reads random bytes into a valid, stack-allocated buffer.
    let ret = unsafe { libc::getrandom(buf.as_mut_ptr() as *mut libc::c_void, 8, 0) };
    if ret != 8 {
        // Fallback: use process-level entropy sources if getrandom fails or short-reads.
        let fallback = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            ^ std::process::id() as u64;
        return fallback;
    }
    u64::from_ne_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_exit_code_success() {
        let marker = "DONE_test123";
        let raw = format!("hello world\n0 {marker}");
        let (code, stdout) = parse_exit_code(&raw, marker);
        assert_eq!(code, 0);
        assert_eq!(stdout, "hello world\n");
    }

    #[test]
    fn test_parse_exit_code_nonzero() {
        let marker = "DONE_test456";
        let raw = format!("error output\n1 {marker}");
        let (code, stdout) = parse_exit_code(&raw, marker);
        assert_eq!(code, 1);
        assert_eq!(stdout, "error output\n");
    }

    #[test]
    fn test_parse_exit_code_missing_marker() {
        let marker = "DONE_missing";
        let raw = "partial output";
        let (code, stdout) = parse_exit_code(raw, marker);
        assert_eq!(code, -1);
        assert_eq!(stdout, "partial output");
    }

    #[test]
    fn test_make_done_marker_unique() {
        let m1 = make_done_marker();
        let m2 = make_done_marker();
        assert_ne!(m1, m2);
        assert!(m1.starts_with("DONE_"));
    }

    #[test]
    fn test_truncate_output_within_limit() {
        let s = "short";
        assert_eq!(truncate_output(s, 100), "short");
    }

    #[test]
    fn test_truncate_output_exceeds_limit() {
        let s = "hello world this is a long string";
        let result = truncate_output(s, 10);
        assert!(result.contains("... [truncated]"));
        assert!(result.len() < s.len() + 20);
    }

    #[test]
    fn test_truncate_output_utf8_boundary() {
        let s = "hello \u{1F600} world";
        let result = truncate_output(s, 8);
        assert!(result.contains("... [truncated]"));
    }
}
