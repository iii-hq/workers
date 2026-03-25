use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Sandbox {
    pub id: String,
    pub language: String,
    pub status: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_us: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VmSnapshot {
    pub regs: Vec<u8>,
    pub sregs: Vec<u8>,
    pub msrs: Vec<u8>,
    pub lapic: Vec<u8>,
    pub ioapic_redirtbl: Vec<u64>,
    pub xcrs: Vec<u8>,
    pub xsave: Vec<u8>,
    pub cpuid_entries: Vec<u8>,
    pub mem_size: usize,
}

pub struct VmInstance {
    pub vm_fd: kvm_ioctls::VmFd,
    pub vcpu_fd: kvm_ioctls::VcpuFd,
    pub mem_ptr: *mut u8,
    pub mem_size: usize,
    pub serial: Serial,
    pub fork_time_us: u64,
    pub _kvm: kvm_ioctls::Kvm,
}

// SAFETY: VmInstance is always accessed behind Arc<Mutex<VmInstance>>,
// which ensures exclusive access to the mutable memory mapping and KVM
// file descriptors. The raw pointer `mem_ptr` points to a per-VM mmap
// region that is not shared with other VmInstances.
unsafe impl Send for VmInstance {}

// SAFETY: All access to VmInstance goes through Arc<Mutex<VmInstance>>.
// The Mutex guarantees that only one thread touches the KVM fds and
// memory mapping at a time.
unsafe impl Sync for VmInstance {}

impl Drop for VmInstance {
    fn drop(&mut self) {
        if !self.mem_ptr.is_null() {
            // SAFETY: mem_ptr was allocated via mmap with mem_size length
            // in fork::fork_from_template. We only unmap once (in Drop).
            unsafe {
                libc::munmap(self.mem_ptr as *mut libc::c_void, self.mem_size);
            }
            self.mem_ptr = std::ptr::null_mut();
        }
    }
}

const MAX_SERIAL_BUFFER: usize = 2 * 1024 * 1024; // 2 MB

pub struct Serial {
    input: VecDeque<u8>,
    output: Vec<u8>,
}

impl Serial {
    pub fn new() -> Self {
        Serial {
            input: VecDeque::new(),
            output: Vec::new(),
        }
    }

    pub fn queue_input(&mut self, data: &[u8]) {
        self.input.extend(data);
    }

    pub fn handle_io_out(&mut self, port: u16, data: &[u8]) {
        if port == 0x3F8 {
            if let Some(&byte) = data.first() {
                if self.output.len() < MAX_SERIAL_BUFFER {
                    self.output.push(byte);
                }
            }
        }
    }

    pub fn handle_io_in(&mut self, port: u16) -> u8 {
        match port {
            0x3F8 => self.input.pop_front().unwrap_or(0),
            0x3FD => {
                let mut lsr: u8 = 0x60;
                if !self.input.is_empty() {
                    lsr |= 0x01;
                }
                lsr
            }
            _ => 0,
        }
    }

    pub fn output_contains(&self, marker: &str) -> bool {
        let out = String::from_utf8_lossy(&self.output);
        out.contains(marker)
    }

    pub fn take_output(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.output).to_string();
        self.output.clear();
        out
    }

    pub fn clear_output(&mut self) {
        self.output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_io() {
        let mut serial = Serial::new();

        serial.queue_input(b"hello");
        assert_eq!(serial.handle_io_in(0x3F8), b'h');
        assert_eq!(serial.handle_io_in(0x3F8), b'e');

        let lsr = serial.handle_io_in(0x3FD);
        assert_ne!(lsr & 0x01, 0, "data ready bit should be set");

        serial.handle_io_out(0x3F8, b"A");
        serial.handle_io_out(0x3F8, b"B");
        assert!(serial.output_contains("AB"));
        let out = serial.take_output();
        assert_eq!(out, "AB");
        assert!(!serial.output_contains("AB"));
    }

    #[test]
    fn test_serial_lsr_empty() {
        let mut serial = Serial::new();
        let lsr = serial.handle_io_in(0x3FD);
        assert_eq!(lsr & 0x01, 0, "data ready bit should be clear");
        assert_ne!(lsr & 0x60, 0, "transmitter empty bits should be set");
    }

    #[test]
    fn test_serial_unknown_port() {
        let mut serial = Serial::new();
        assert_eq!(serial.handle_io_in(0x400), 0);
        serial.handle_io_out(0x400, b"X");
        assert!(serial.take_output().is_empty());
    }

    #[test]
    fn test_serial_empty_data_out() {
        let mut serial = Serial::new();
        serial.handle_io_out(0x3F8, &[]);
        assert!(serial.take_output().is_empty());
    }

    #[test]
    fn test_serial_buffer_bounded() {
        let mut serial = Serial::new();
        for _ in 0..MAX_SERIAL_BUFFER + 100 {
            serial.handle_io_out(0x3F8, &[b'A']);
        }
        assert_eq!(serial.output.len(), MAX_SERIAL_BUFFER);
    }

    #[test]
    fn test_serial_clear_output() {
        let mut serial = Serial::new();
        serial.handle_io_out(0x3F8, b"X");
        serial.clear_output();
        assert!(serial.take_output().is_empty());
    }

    #[test]
    fn test_sandbox_serialization() {
        let sandbox = Sandbox {
            id: "sbx-001".to_string(),
            language: "python".to_string(),
            status: "running".to_string(),
            created_at: 1700000000,
            expires_at: 1700003600,
        };
        let json = serde_json::to_string(&sandbox).unwrap();
        let parsed: Sandbox = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "sbx-001");
        assert_eq!(parsed.language, "python");
    }

    #[test]
    fn test_exec_result_serialization() {
        let result = ExecResult {
            exit_code: 0,
            stdout: "hello world\n".to_string(),
            stderr: String::new(),
            duration_us: 1234,
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ExecResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.exit_code, 0);
        assert_eq!(parsed.stdout, "hello world\n");
        assert_eq!(parsed.duration_us, 1234);
    }
}
