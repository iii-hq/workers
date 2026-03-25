use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::VmSnapshot;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedVmState {
    pub regs: Vec<u64>,
    pub sregs: SavedSregs,
    pub msrs: Vec<SavedMsr>,
    pub lapic: Vec<u8>,
    pub ioapic_redirtbl: Vec<u64>,
    pub xcrs: Vec<SavedXcr>,
    pub xsave: Vec<u8>,
    pub cpuid: Vec<SavedCpuidEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedSregs {
    pub cs: SavedSegment,
    pub ds: SavedSegment,
    pub es: SavedSegment,
    pub fs: SavedSegment,
    pub gs: SavedSegment,
    pub ss: SavedSegment,
    pub tr: SavedSegment,
    pub ldt: SavedSegment,
    pub gdt: SavedDtable,
    pub idt: SavedDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub apic_base: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedDtable {
    pub base: u64,
    pub limit: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedMsr {
    pub index: u32,
    pub data: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedXcr {
    pub xcr: u32,
    pub value: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedCpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

pub fn load_vmstate(path: &str) -> Result<SavedVmState> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read vmstate from {path}"))?;
    let state: SavedVmState = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse vmstate JSON from {path}"))?;
    Ok(state)
}

impl SavedVmState {
    pub fn to_snapshot(&self, mem_size: usize) -> VmSnapshot {
        let regs = serde_json::to_vec(&self.regs).unwrap_or_default();
        let sregs = serde_json::to_vec(&self.sregs).unwrap_or_default();
        let msrs = serde_json::to_vec(&self.msrs).unwrap_or_default();
        let xcrs = serde_json::to_vec(&self.xcrs).unwrap_or_default();
        let cpuid_entries = serde_json::to_vec(&self.cpuid).unwrap_or_default();

        VmSnapshot {
            regs,
            sregs,
            msrs,
            lapic: self.lapic.clone(),
            ioapic_redirtbl: self.ioapic_redirtbl.clone(),
            xcrs,
            xsave: self.xsave.clone(),
            cpuid_entries,
            mem_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_state() -> SavedVmState {
        SavedVmState {
            regs: vec![0; 23],
            sregs: SavedSregs {
                cs: make_segment(),
                ds: make_segment(),
                es: make_segment(),
                fs: make_segment(),
                gs: make_segment(),
                ss: make_segment(),
                tr: make_segment(),
                ldt: make_segment(),
                gdt: SavedDtable { base: 0, limit: 0 },
                idt: SavedDtable { base: 0, limit: 0 },
                cr0: 0x80050033,
                cr2: 0,
                cr3: 0x1000,
                cr4: 0x3420,
                efer: 0xD01,
                apic_base: 0xFEE00900,
            },
            msrs: vec![SavedMsr {
                index: 0x174,
                data: 0,
            }],
            lapic: vec![0; 1024],
            ioapic_redirtbl: vec![0; 24],
            xcrs: vec![SavedXcr { xcr: 0, value: 7 }],
            xsave: vec![0; 4096],
            cpuid: vec![SavedCpuidEntry {
                function: 0,
                index: 0,
                flags: 0,
                eax: 0x16,
                ebx: 0,
                ecx: 0,
                edx: 0,
            }],
        }
    }

    fn make_segment() -> SavedSegment {
        SavedSegment {
            base: 0,
            limit: 0xFFFFFFFF,
            selector: 0x10,
            type_: 11,
            present: 1,
            dpl: 0,
            db: 0,
            s: 1,
            l: 1,
            g: 1,
            avl: 0,
        }
    }

    #[test]
    fn test_to_snapshot() {
        let state = make_test_state();
        let snapshot = state.to_snapshot(256 * 1024 * 1024);
        assert_eq!(snapshot.mem_size, 256 * 1024 * 1024);
        assert!(!snapshot.regs.is_empty());
        assert!(!snapshot.sregs.is_empty());
        assert_eq!(snapshot.lapic.len(), 1024);
        assert_eq!(snapshot.ioapic_redirtbl.len(), 24);
    }

    #[test]
    fn test_saved_vmstate_roundtrip() {
        let state = make_test_state();
        let json = serde_json::to_string(&state).unwrap();
        let parsed: SavedVmState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.regs.len(), state.regs.len());
        assert_eq!(parsed.sregs.cr0, state.sregs.cr0);
        assert_eq!(parsed.msrs.len(), state.msrs.len());
    }
}
