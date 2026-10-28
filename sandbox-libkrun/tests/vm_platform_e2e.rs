//! End-to-end integration tests for the VM platform stack.
//!
//! These tests require actual KVM (Linux) or Hypervisor.framework (macOS) access.
//! They are `#[ignore]` by default -- run with: `cargo test --test vm_platform_e2e -- --ignored`
//!
//! Requirements validated: PLAT-01 (Linux KVM), PLAT-02 (macOS Hypervisor.framework)

#![cfg(not(target_os = "windows"))]

#[cfg(target_os = "linux")]
use std::path::Path;
use std::process::Command;

// ── Pre-flight checks ──────────────────────────────────────────────

/// Documents KVM status on Linux. Does not fail if KVM is missing --
/// the VM boot test will skip instead.
#[test]
#[cfg(target_os = "linux")]
fn test_kvm_available() {
    let kvm = Path::new("/dev/kvm");
    if !kvm.exists() {
        eprintln!("WARNING: /dev/kvm not found -- VM boot tests will be skipped");
        return;
    }
    // Try opening for read/write (required by libkrun)
    match std::fs::File::options().read(true).write(true).open(kvm) {
        Ok(_) => eprintln!("KVM is available and accessible"),
        Err(e) => eprintln!("WARNING: /dev/kvm exists but not accessible: {}", e),
    }
}

/// Validates that libkrunfw resolution logic can locate the firmware file.
/// Skips if no firmware is in place yet (not a failure).
#[test]
#[ignore] // Requires firmware in ~/.iii/lib/ or a system path
fn test_libkrunfw_resolves() {
    // Check known locations for the firmware file
    let filename = if cfg!(target_os = "macos") {
        "libkrunfw.5.dylib"
    } else {
        "libkrunfw.so.5.6.2"
    };

    let candidates: Vec<std::path::PathBuf> = [
        dirs::home_dir().map(|h| h.join(".iii").join("lib").join(filename)),
        Some(std::path::PathBuf::from("/usr/local/lib").join(filename)),
        Some(std::path::PathBuf::from("/usr/lib").join(filename)),
    ]
    .into_iter()
    .flatten()
    .collect();

    let found = candidates.iter().any(|p| p.exists());
    if !found {
        eprintln!(
            "WARNING: libkrunfw not found in any known location. Checked: {:?}",
            candidates
        );
        eprintln!(
            "A sandbox-libkrun built with --features embed-libkrunfw extracts it to ~/.iii/lib on its first VM boot"
        );
        return;
    }
    eprintln!("libkrunfw found in a known location");
}

// ── Main VM boot test ──────────────────────────────────────────────

/// End-to-end test that spawns `sandbox-libkrun __vm-boot` and verifies the full boot path:
/// 1. libkrunfw resolution (firmware library found)
/// 2. VM boots without crash (exit code 0 or clean exit)
/// 3. Init mounts /proc (VM_BOOT_OK marker in output)
/// 4. Worker process exits cleanly
///
/// Requires KVM (Linux) or Hypervisor.framework (macOS).
/// Run with: `cargo test --test vm_platform_e2e -- --ignored`
#[test]
#[ignore] // Requires KVM (Linux) or Hypervisor.framework (macOS)
fn test_vm_boot_platform() {
    // Skip if KVM not available on Linux
    #[cfg(target_os = "linux")]
    {
        let kvm = Path::new("/dev/kvm");
        if !kvm.exists() {
            eprintln!("Skipping: /dev/kvm not found");
            return;
        }
        if std::fs::File::options()
            .read(true)
            .write(true)
            .open(kvm)
            .is_err()
        {
            eprintln!("Skipping: /dev/kvm not accessible");
            return;
        }
    }
    // Library path `__vm-boot` gets, for its libkrunfw soname fallback.
    let lib_paths: Vec<String> = [
        dirs::home_dir().map(|h| h.join(".iii").join("lib").to_string_lossy().to_string()),
        Some("/usr/local/lib".to_string()),
        Some("/usr/lib".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();

    // The other prerequisites are checked here, not read back from the boot's
    // output: past this point every failure is a real boot failure.
    if !libkrunfw_loadable(&lib_paths) {
        eprintln!(
            "Skipping: libkrunfw not found (III_LIBKRUNFW_PATH, ~/.iii/lib or the loader's search path)"
        );
        return;
    }
    if !iii_filesystem::init::has_init() {
        eprintln!("Skipping: no embedded iii-init (build with --features embed-init)");
        return;
    }

    // 1. Find the sandbox-libkrun binary
    let bin_path = env!("CARGO_BIN_EXE_sandbox-libkrun");

    // 2. Create a minimal rootfs for the test.
    //    This avoids needing to pull an OCI image (which needs network + registry access).
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    let rootfs = tmp.path().join("rootfs");

    // Create minimal rootfs structure
    for dir in &["bin", "lib", "lib64", "etc", "proc", "sys", "dev", "tmp"] {
        std::fs::create_dir_all(rootfs.join(dir)).unwrap();
    }

    // The rootfs has no dynamic loader or libs, so only a static Linux shell
    // runs in it (a static busybox; never macOS's Mach-O /bin/sh).
    let Some(sh) = ["/usr/bin/busybox", "/bin/busybox", "/bin/sh"]
        .into_iter()
        .find(|p| std::fs::read(p).is_ok_and(|elf| is_static_elf(&elf)))
    else {
        eprintln!("Skipping: no static Linux shell for the test rootfs (install busybox-static)");
        return;
    };
    std::fs::copy(sh, rootfs.join("bin/sh")).expect("failed to copy the shell into the rootfs");

    // 3. Spawn sandbox-libkrun __vm-boot with a simple command that proves init works.
    //    The command: /bin/sh -c "test -d /proc/self && echo VM_BOOT_OK"
    //    If init mounted /proc correctly, /proc/self exists and we see VM_BOOT_OK.
    let mut cmd = Command::new(bin_path);
    cmd.arg("__vm-boot")
        .arg("--rootfs")
        .arg(&rootfs)
        .arg("--exec")
        .arg("/bin/sh")
        .arg("--arg")
        .arg("-c")
        .arg("--arg")
        .arg("test -d /proc/self && echo VM_BOOT_OK || echo VM_BOOT_FAIL")
        .arg("--vcpus")
        .arg("1")
        .arg("--ram")
        .arg("256");

    // Set library path for libkrun resolution
    let lib_path_var = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    cmd.env(lib_path_var, lib_paths.join(":"));

    let output = cmd
        .output()
        .expect("failed to spawn sandbox-libkrun __vm-boot");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    eprintln!("--- stdout ---\n{}", stdout);
    eprintln!("--- stderr ---\n{}", stderr);

    assert!(
        output.status.success(),
        "VM boot failed with exit code {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout,
        stderr
    );

    // Verify init mounted /proc and the worker command executed successfully
    assert!(
        stdout.contains("VM_BOOT_OK") || stderr.contains("VM_BOOT_OK"),
        "Expected VM_BOOT_OK in output (init should mount /proc). stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

/// Whether `__vm-boot` can load libkrunfw: the file it resolves itself, or else
/// the soname msb_krun hands the dynamic loader, with `lib_dirs` searched first.
fn libkrunfw_loadable(lib_dirs: &[String]) -> bool {
    if sandbox_libkrun::cli::vm_boot::resolve_krunfw_file_path().is_some() {
        return true;
    }
    let abi = sandbox_libkrun::cli::firmware::constants::LIBKRUNFW_ABI;
    let soname = if cfg!(target_os = "macos") {
        format!("libkrunfw.{abi}.dylib")
    } else {
        format!("libkrunfw.so.{abi}")
    };
    lib_dirs
        .iter()
        .map(|dir| format!("{dir}/{soname}"))
        .chain([soname.clone()])
        .any(|name| {
            let name = std::ffi::CString::new(name).expect("library path has a NUL");
            // SAFETY: dlopen/dlclose of a NUL-terminated path; no symbol is called.
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
            !handle.is_null() && unsafe { libc::dlclose(handle) } == 0
        })
}

/// Whether `elf` is a 64-bit little-endian ELF without a PT_INTERP segment,
/// so it runs without a dynamic loader.
fn is_static_elf(elf: &[u8]) -> bool {
    if elf.len() < 64 || !elf.starts_with(b"\x7fELF") || elf[4] != 2 || elf[5] != 1 {
        return false;
    }
    let u16_at = |at: usize| u16::from_le_bytes([elf[at], elf[at + 1]]) as usize;
    let phoff = u64::from_le_bytes(elf[0x20..0x28].try_into().unwrap());
    let (phentsize, phnum) = (u16_at(0x36), u16_at(0x38));
    let Some(phdrs) = usize::try_from(phoff).ok().and_then(|at| elf.get(at..)) else {
        return false;
    };
    const PT_INTERP: [u8; 4] = 3u32.to_le_bytes();
    (0..phnum).all(|i| {
        phdrs
            .get(i * phentsize..i * phentsize + 4)
            .is_some_and(|p_type| p_type != PT_INTERP)
    })
}

#[test]
fn is_static_elf_needs_elf64_le_without_pt_interp() {
    // ELF64 LE header plus one 56-byte program header at offset 64.
    let elf = |p_type: u32| {
        let mut b = vec![0u8; 64 + 56];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
        b[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        b[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&p_type.to_le_bytes());
        b
    };
    assert!(is_static_elf(&elf(1)), "PT_LOAD only is static");
    assert!(!is_static_elf(&elf(3)), "PT_INTERP is dynamic");
    assert!(!is_static_elf(&elf(1)[..66]), "a truncated program header");
    let mut elf32 = elf(1);
    elf32[4] = 1;
    assert!(!is_static_elf(&elf32), "ELF32");
    let mut macho = elf(1);
    macho[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    assert!(!is_static_elf(&macho), "Mach-O");
}
