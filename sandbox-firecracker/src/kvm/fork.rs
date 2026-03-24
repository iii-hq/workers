use anyhow::{bail, Context, Result};
use kvm_ioctls::Kvm;
use std::time::Instant;

use super::cpu;
use super::template::Template;
use crate::types::{Serial, VmInstance};

pub fn fork_from_template(template: &Template) -> Result<VmInstance> {
    let start = Instant::now();

    let kvm = Kvm::new().context("failed to open /dev/kvm")?;
    let vm_fd = kvm.create_vm().context("failed to create VM")?;

    vm_fd.create_irq_chip().context("failed to create IRQ chip")?;

    let mut pit_config = kvm_bindings::kvm_pit_config::default();
    pit_config.flags = 0;
    vm_fd
        .create_pit2(pit_config)
        .context("failed to create PIT2")?;

    for (i, &entry) in template.snapshot.ioapic_redirtbl.iter().enumerate() {
        if entry != 0 {
            tracing::trace!(pin = i, entry, "restoring IOAPIC redirect entry");
        }
    }

    // SAFETY: We mmap the template memfd with MAP_PRIVATE to get CoW semantics.
    // Each forked VM gets its own copy-on-write view of the template memory.
    // MAP_NORESERVE avoids reserving swap for the full mapping upfront.
    let mem_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            template.mem_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_NORESERVE,
            template.memfd,
            0,
        )
    };

    if mem_ptr == libc::MAP_FAILED {
        bail!(
            "mmap MAP_PRIVATE failed: {}",
            std::io::Error::last_os_error()
        );
    }

    // SAFETY: vm_fd is a valid KVM VM. mem_ptr is a valid mmap region of
    // template.mem_size bytes obtained above. Setting slot 0 with
    // guest_phys_addr=0 maps the entire guest physical address space.
    // KVM_MEM_LOG_DIRTY_PAGES is not set — we do not track dirty pages.
    let mem_region = kvm_bindings::kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: template.mem_size as u64,
        userspace_addr: mem_ptr as u64,
    };
    unsafe {
        vm_fd
            .set_user_memory_region(mem_region)
            .context("failed to set user memory region")?;
    }

    let vcpu_fd = vm_fd.create_vcpu(0).context("failed to create vCPU")?;

    cpu::restore_cpu_state(&vcpu_fd, &template.snapshot)
        .context("failed to restore CPU state")?;

    let fork_time_us = start.elapsed().as_micros() as u64;

    tracing::debug!(fork_time_us, "VM forked from template");

    Ok(VmInstance {
        vm_fd,
        vcpu_fd,
        mem_ptr: mem_ptr as *mut u8,
        mem_size: template.mem_size,
        serial: Serial::new(),
        fork_time_us,
        _kvm: kvm,
    })
}
