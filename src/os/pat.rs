// pat.rs - PAT (Page Attribute Table) write-combining for framebuffer memory
//
// Copyright (C) 2026 Eric Klavins
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use core::arch::asm;

const PAT_MSR: u32 = 0x277;
const PAT_WC: u64 = 0x01;

const PTE_PRESENT: u64 = 1 << 0;
const PTE_PWT: u64 = 1 << 3;
const PTE_PCD: u64 = 1 << 4;
const PTE_PS: u64 = 1 << 7;      // large page flag (PDPT/PD)
const PTE_PAT_4K: u64 = 1 << 7;  // PAT bit for 4KB pages
const PTE_PAT_LARGE: u64 = 1 << 12; // PAT bit for 2MB/1GB pages

const CR0_WP: u64 = 1 << 16;
const PTE_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe { asm!("rdmsr", out("eax") lo, out("edx") hi, in("ecx") msr, options(nomem, nostack)); }
    ((hi as u64) << 32) | lo as u64
}

fn wrmsr(msr: u32, val: u64) {
    unsafe {
        asm!("wrmsr",
            in("ecx") msr,
            in("eax") val as u32,
            in("edx") (val >> 32) as u32,
            options(nomem, nostack));
    }
}

fn read_cr0() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr0", out(reg) val, options(nomem, nostack)); }
    val
}

fn write_cr0(val: u64) {
    unsafe { asm!("mov cr0, {}", in(reg) val, options(nomem, nostack)); }
}

fn read_cr3() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr3", out(reg) val, options(nomem, nostack)); }
    val
}

fn invlpg(addr: u64) {
    unsafe { asm!("invlpg [{}]", in(reg) addr, options(nostack)); }
}

/// Set PAT slot 1 to Write-Combining. Call once at boot.
pub fn pat_init() {
    let mut pat = rdmsr(PAT_MSR);
    pat &= !(0xFFu64 << 8);       // clear slot 1
    pat |= PAT_WC << 8;           // slot 1 = WC
    wrmsr(PAT_MSR, pat);
}

/// Set PTE to use PAT slot 1 (WC) for a 4KB page.
fn set_wc_4k(entry: &mut u64) {
    *entry |= PTE_PWT;
    *entry &= !PTE_PCD;
    *entry &= !PTE_PAT_4K;
}

/// Set PTE to use PAT slot 1 (WC) for a 2MB or 1GB large page.
fn set_wc_large(entry: &mut u64) {
    *entry |= PTE_PWT;
    *entry &= !PTE_PCD;
    *entry &= !PTE_PAT_LARGE;
}

/// Walk the 4-level page table and set write-combining on the given
/// physical address range. Temporarily clears CR0.WP to modify
/// UEFI's write-protected page tables.
pub fn pat_set_write_combining(phys_addr: u64, size: u64) {
    let cr3 = read_cr3();
    let pml4 = (cr3 & PTE_ADDR_MASK) as *mut u64;

    let cr0 = read_cr0();
    write_cr0(cr0 & !CR0_WP);

    let end = phys_addr + size;
    let mut addr = phys_addr;

    while addr < end {
        // PML4 index
        let pml4_idx = ((addr >> 39) & 0x1FF) as usize;
        let pml4_entry = unsafe { &mut *pml4.add(pml4_idx) };
        if *pml4_entry & PTE_PRESENT == 0 {
            addr = (addr + (1u64 << 39)) & !((1u64 << 39) - 1);
            continue;
        }

        let pdpt = (*pml4_entry & PTE_ADDR_MASK) as *mut u64;
        let pdpt_idx = ((addr >> 30) & 0x1FF) as usize;
        let pdpt_entry = unsafe { &mut *pdpt.add(pdpt_idx) };
        if *pdpt_entry & PTE_PRESENT == 0 {
            addr = (addr + (1u64 << 30)) & !((1u64 << 30) - 1);
            continue;
        }

        // 1GB large page
        if *pdpt_entry & PTE_PS != 0 {
            set_wc_large(pdpt_entry);
            invlpg(addr);
            addr = (addr + (1u64 << 30)) & !((1u64 << 30) - 1);
            continue;
        }

        let pd = (*pdpt_entry & PTE_ADDR_MASK) as *mut u64;
        let pd_idx = ((addr >> 21) & 0x1FF) as usize;
        let pd_entry = unsafe { &mut *pd.add(pd_idx) };
        if *pd_entry & PTE_PRESENT == 0 {
            addr = (addr + (1u64 << 21)) & !((1u64 << 21) - 1);
            continue;
        }

        // 2MB large page
        if *pd_entry & PTE_PS != 0 {
            set_wc_large(pd_entry);
            invlpg(addr);
            addr = (addr + (1u64 << 21)) & !((1u64 << 21) - 1);
            continue;
        }

        let pt = (*pd_entry & PTE_ADDR_MASK) as *mut u64;
        let pt_idx = ((addr >> 12) & 0x1FF) as usize;
        let pt_entry = unsafe { &mut *pt.add(pt_idx) };
        if *pt_entry & PTE_PRESENT != 0 {
            set_wc_4k(pt_entry);
            invlpg(addr);
        }
        addr += 1u64 << 12;
    }

    write_cr0(cr0);
}
