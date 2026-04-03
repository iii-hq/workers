use anyhow::{Context, Result};
use kvm_bindings::{kvm_mp_state, KVM_MP_STATE_RUNNABLE};
use kvm_ioctls::VcpuFd;

use crate::kvm::vmstate::{SavedCpuidEntry, SavedMsr, SavedSregs, SavedXcr};
use crate::types::VmSnapshot;

pub fn restore_cpu_state(vcpu: &VcpuFd, snapshot: &VmSnapshot) -> Result<()> {
    restore_cpuid(vcpu, &snapshot.cpuid_entries)?;
    restore_sregs(vcpu, &snapshot.sregs)?;
    restore_xcrs(vcpu, &snapshot.xcrs)?;
    restore_xsave(vcpu, &snapshot.xsave)?;
    restore_regs(vcpu, &snapshot.regs)?;
    restore_lapic(vcpu, &snapshot.lapic)?;
    restore_msrs(vcpu, &snapshot.msrs)?;
    restore_mp_state(vcpu)?;
    Ok(())
}

fn restore_cpuid(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    let entries: Vec<SavedCpuidEntry> =
        serde_json::from_slice(data).context("failed to deserialize CPUID entries")?;

    let mut kvm_entries = Vec::with_capacity(entries.len());
    for e in &entries {
        kvm_entries.push(kvm_bindings::kvm_cpuid_entry2 {
            function: e.function,
            index: e.index,
            flags: e.flags,
            eax: e.eax,
            ebx: e.ebx,
            ecx: e.ecx,
            edx: e.edx,
            padding: [0; 3],
        });
    }

    let mut cpuid =
        kvm_bindings::CpuId::new(kvm_entries.len()).context("failed to allocate CpuId")?;
    let cpuid_slice = cpuid.as_mut_slice();
    for (dst, src) in cpuid_slice.iter_mut().zip(kvm_entries.iter()) {
        *dst = *src;
    }

    vcpu.set_cpuid2(&cpuid).context("set_cpuid2 failed")?;
    Ok(())
}

fn restore_sregs(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    let saved: SavedSregs = serde_json::from_slice(data).context("failed to deserialize sregs")?;

    let mut sregs = vcpu.get_sregs().context("get_sregs failed")?;

    apply_segment(&mut sregs.cs, &saved.cs);
    apply_segment(&mut sregs.ds, &saved.ds);
    apply_segment(&mut sregs.es, &saved.es);
    apply_segment(&mut sregs.fs, &saved.fs);
    apply_segment(&mut sregs.gs, &saved.gs);
    apply_segment(&mut sregs.ss, &saved.ss);
    apply_segment(&mut sregs.tr, &saved.tr);
    apply_segment(&mut sregs.ldt, &saved.ldt);
    sregs.gdt.base = saved.gdt.base;
    sregs.gdt.limit = saved.gdt.limit;
    sregs.idt.base = saved.idt.base;
    sregs.idt.limit = saved.idt.limit;
    sregs.cr0 = saved.cr0;
    sregs.cr2 = saved.cr2;
    sregs.cr3 = saved.cr3;
    sregs.cr4 = saved.cr4;
    sregs.efer = saved.efer;
    sregs.apic_base = saved.apic_base;

    vcpu.set_sregs(&sregs).context("set_sregs failed")?;
    Ok(())
}

fn apply_segment(dst: &mut kvm_bindings::kvm_segment, src: &crate::kvm::vmstate::SavedSegment) {
    dst.base = src.base;
    dst.limit = src.limit;
    dst.selector = src.selector;
    dst.type_ = src.type_;
    dst.present = src.present;
    dst.dpl = src.dpl;
    dst.db = src.db;
    dst.s = src.s;
    dst.l = src.l;
    dst.g = src.g;
    dst.avl = src.avl;
}

fn restore_xcrs(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    let saved: Vec<SavedXcr> =
        serde_json::from_slice(data).context("failed to deserialize XCRs")?;

    if saved.is_empty() {
        return Ok(());
    }

    let mut xcrs = vcpu.get_xcrs().context("get_xcrs failed")?;
    let max = xcrs.xcrs.len();
    anyhow::ensure!(
        saved.len() <= max,
        "snapshot has {} XCRs but host supports at most {}",
        saved.len(),
        max
    );
    for (i, s) in saved.iter().enumerate() {
        xcrs.xcrs[i].xcr = s.xcr;
        xcrs.xcrs[i].value = s.value;
    }
    xcrs.nr_xcrs = saved.len() as u32;

    vcpu.set_xcrs(&xcrs).context("set_xcrs failed")?;
    Ok(())
}

fn restore_xsave(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }

    let mut xsave = vcpu.get_xsave().context("get_xsave failed")?;
    let xsave_region = &mut xsave.region;
    let capacity = xsave_region.len() * 4;
    anyhow::ensure!(
        data.len() <= capacity,
        "xsave data ({} bytes) exceeds host capacity ({} bytes)",
        data.len(),
        capacity
    );
    let copy_len = data.len();

    // SAFETY: We copy raw bytes into the xsave region. copy_len is bounded
    // by the smaller of data.len() and the xsave region capacity.
    unsafe {
        std::ptr::copy_nonoverlapping(
            data.as_ptr(),
            xsave_region.as_mut_ptr() as *mut u8,
            copy_len,
        );
    }

    vcpu.set_xsave(&xsave).context("set_xsave failed")?;
    Ok(())
}

fn restore_regs(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    let values: Vec<u64> = serde_json::from_slice(data).context("failed to deserialize regs")?;
    anyhow::ensure!(
        values.len() >= 17,
        "snapshot regs has {} values, need at least 17 (rax..rip)",
        values.len()
    );

    let mut regs = vcpu.get_regs().context("get_regs failed")?;

    regs.rax = values[0];
    regs.rbx = values[1];
    regs.rcx = values[2];
    regs.rdx = values[3];
    regs.rsi = values[4];
    regs.rdi = values[5];
    regs.rsp = values[6];
    regs.rbp = values[7];
    regs.r8 = values[8];
    regs.r9 = values[9];
    regs.r10 = values[10];
    regs.r11 = values[11];
    regs.r12 = values[12];
    regs.r13 = values[13];
    regs.r14 = values[14];
    regs.r15 = values[15];
    regs.rip = values[16];
    if values.len() >= 18 {
        regs.rflags = values[17];
    }

    vcpu.set_regs(&regs).context("set_regs failed")?;
    Ok(())
}

fn restore_lapic(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }

    let mut lapic = vcpu.get_lapic().context("get_lapic failed")?;
    let regs = &mut lapic.regs;
    anyhow::ensure!(
        data.len() <= regs.len(),
        "lapic data ({} bytes) exceeds host register size ({} bytes)",
        data.len(),
        regs.len()
    );
    let copy_len = data.len();
    // SAFETY: lapic.regs is [i8] on x86_64 Linux but we write raw bytes.
    // The KVM API treats this as an opaque byte buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), regs.as_mut_ptr() as *mut u8, copy_len);
    }

    vcpu.set_lapic(&lapic).context("set_lapic failed")?;
    Ok(())
}

fn restore_msrs(vcpu: &VcpuFd, data: &[u8]) -> Result<()> {
    let saved: Vec<SavedMsr> =
        serde_json::from_slice(data).context("failed to deserialize MSRs")?;

    if saved.is_empty() {
        return Ok(());
    }

    let mut msrs = kvm_bindings::Msrs::new(saved.len()).context("failed to allocate Msrs")?;
    let entries = msrs.as_mut_slice();
    for (i, s) in saved.iter().enumerate() {
        if i < entries.len() {
            entries[i].index = s.index;
            entries[i].data = s.data;
        }
    }

    let written = vcpu.set_msrs(&msrs).context("set_msrs failed")?;
    anyhow::ensure!(
        written as usize == saved.len(),
        "set_msrs wrote {}/{} entries",
        written,
        saved.len()
    );
    Ok(())
}

fn restore_mp_state(vcpu: &VcpuFd) -> Result<()> {
    let mp_state = kvm_mp_state {
        mp_state: KVM_MP_STATE_RUNNABLE,
    };
    vcpu.set_mp_state(mp_state).context("set_mp_state failed")?;
    Ok(())
}
