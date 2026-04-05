// ahci.rs - AHCI SATA disk driver
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

use crate::pci::{pci_find_device, pci_read_bar, pci_enable_device};
use core::ptr;

const SECTOR_SIZE: usize = 512;
const AHCI_MAX_PORTS: usize = 32;

// ATA commands
const ATA_CMD_READ_DMA_EX: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EX: u8 = 0x35;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

// HBA port CMD register bits
const HBA_PORT_CMD_ST: u32 = 1 << 0;
const HBA_PORT_CMD_FRE: u32 = 1 << 4;
const HBA_PORT_CMD_FR: u32 = 1 << 14;
const HBA_PORT_CMD_CR: u32 = 1 << 15;

// HBA global control
const HBA_GHC_AE: u32 = 1 << 31;

// Task file data bits
const ATA_DEV_BUSY: u32 = 0x80;
const ATA_DEV_DRQ: u32 = 0x08;

// SATA status — device detected and phy communication established
const SSTS_DET_PRESENT: u32 = 0x3;

// FIS type
const FIS_TYPE_REG_H2D: u8 = 0x27;

// DMA buffers — must be aligned and in low memory
// Per port: 1024 (cmd list) + 256 (FIS) + 256 (cmd table) = 1536, padded to 2048
#[repr(C, align(4096))]
struct DmaArea([u8; AHCI_MAX_PORTS * 2048]);

static mut DMA: DmaArea = DmaArea([0; AHCI_MAX_PORTS * 2048]);

#[repr(C, align(512))]
struct SectorBuf([u8; SECTOR_SIZE]);

static mut SECTOR_BUF: SectorBuf = SectorBuf([0; SECTOR_SIZE]);

// Per-port state
static mut PORT_ACTIVE: [bool; AHCI_MAX_PORTS] = [false; AHCI_MAX_PORTS];
static mut PORT_SECTORS: [u64; AHCI_MAX_PORTS] = [0; AHCI_MAX_PORTS];
static mut ACTIVE_PORT: i32 = -1;

// HBA base address
static mut HBA: *mut u8 = core::ptr::null_mut();

// HBA global register access
fn hba_ptr() -> *mut u8 {
    unsafe { core::ptr::read_volatile(&raw const HBA) }
}

fn hba_read(offset: usize) -> u32 {
    unsafe { ptr::read_volatile(hba_ptr().add(offset) as *const u32) }
}

fn hba_write(offset: usize, val: u32) {
    unsafe { ptr::write_volatile(hba_ptr().add(offset) as *mut u32, val) }
}

fn port_base(port: usize) -> *mut u8 {
    unsafe { hba_ptr().add(0x100 + port * 0x80) }
}

fn port_read(port: usize, offset: usize) -> u32 {
    unsafe { ptr::read_volatile(port_base(port).add(offset) as *const u32) }
}

fn port_write(port: usize, offset: usize, val: u32) {
    unsafe { ptr::write_volatile(port_base(port).add(offset) as *mut u32, val) }
}

// DMA buffer helpers
fn dma_base() -> *mut u8 {
    (&raw mut DMA) as *mut u8
}

fn sbuf() -> *mut u8 {
    (&raw mut SECTOR_BUF) as *mut u8
}

fn cmd_list_ptr(port: usize) -> *mut u8 {
    unsafe { dma_base().add(port * 2048) }
}

fn fis_area_ptr(port: usize) -> *mut u8 {
    unsafe { dma_base().add(port * 2048 + 1024) }
}

fn cmd_tbl_ptr(port: usize) -> *mut u8 {
    unsafe { dma_base().add(port * 2048 + 1024 + 256) }
}

fn port_stop(port: usize) {
    let mut cmd = port_read(port, 0x18); // CMD register
    cmd &= !HBA_PORT_CMD_ST;
    cmd &= !HBA_PORT_CMD_FRE;
    port_write(port, 0x18, cmd);
    // Wait for FR and CR to clear
    for _ in 0..100000u32 {
        let cmd = port_read(port, 0x18);
        if cmd & (HBA_PORT_CMD_FR | HBA_PORT_CMD_CR) == 0 {
            break;
        }
    }
}

fn port_start(port: usize) {
    // Wait for CR to clear
    for _ in 0..100000u32 {
        if port_read(port, 0x18) & HBA_PORT_CMD_CR == 0 { break; }
    }
    let cmd = port_read(port, 0x18);
    port_write(port, 0x18, cmd | HBA_PORT_CMD_FRE | HBA_PORT_CMD_ST);
}

fn init_port(port: usize) {
    port_stop(port);

    // Command list
    let cl = cmd_list_ptr(port);
    let cl_addr = cl as u64;
    unsafe { ptr::write_bytes(cl, 0, 1024); }
    port_write(port, 0x00, cl_addr as u32);       // CLB
    port_write(port, 0x04, (cl_addr >> 32) as u32); // CLBU

    // FIS receive area
    let fb = fis_area_ptr(port);
    let fb_addr = fb as u64;
    unsafe { ptr::write_bytes(fb, 0, 256); }
    port_write(port, 0x08, fb_addr as u32);       // FB
    port_write(port, 0x0C, (fb_addr >> 32) as u32); // FBU

    // Command table for slot 0
    let ct = cmd_tbl_ptr(port);
    let ct_addr = ct as u64;
    unsafe {
        ptr::write_bytes(ct, 0, 256);
        // Write command table base into command header 0 (offset 8 and 12 in header)
        let hdr = cl as *mut u32;
        ptr::write_volatile(hdr.add(2), ct_addr as u32);       // CTBA
        ptr::write_volatile(hdr.add(3), (ct_addr >> 32) as u32); // CTBAU
    }

    // Clear errors and interrupts
    port_write(port, 0x30, 0xFFFFFFFF); // SERR
    port_write(port, 0x10, 0xFFFFFFFF); // IS

    port_start(port);
}

fn port_wait(port: usize, timeout: u32) -> bool {
    for _ in 0..timeout {
        if port_read(port, 0x38) & 1 == 0 { // CI bit 0 cleared
            return true;
        }
        let tfd = port_read(port, 0x20);
        if tfd & 0x01 != 0 { // Error bit in TFD
            return false;
        }
    }
    false
}

fn issue_cmd(port: usize, command: u8, lba: u64, write: bool) -> bool {
    // Wait for device not busy
    for _ in 0..100000u32 {
        if port_read(port, 0x20) & (ATA_DEV_BUSY | ATA_DEV_DRQ) == 0 { break; }
    }

    let cl = cmd_list_ptr(port) as *mut u32;
    let ct = cmd_tbl_ptr(port);

    unsafe {
        // Command header flags: CFL = 5 dwords (FIS_REG_H2D = 20 bytes / 4)
        let mut flags: u16 = 5;
        if write { flags |= 1 << 6; }
        ptr::write_volatile(cl as *mut u16, flags);
        ptr::write_volatile((cl as *mut u16).add(1), 1); // PRDTL = 1 entry

        // Clear PRD byte count
        ptr::write_volatile(cl.add(1), 0); // PRDBC

        // Set up PRDT entry at offset 0x80 in command table
        let buf_addr = sbuf() as u64;
        let prdt = ct.add(0x80) as *mut u32;
        ptr::write_volatile(prdt.add(0), buf_addr as u32);       // DBA
        ptr::write_volatile(prdt.add(1), (buf_addr >> 32) as u32); // DBAU
        ptr::write_volatile(prdt.add(2), 0);                      // Reserved
        ptr::write_volatile(prdt.add(3), 511);                    // DBC = 512-1

        // Build FIS_REG_H2D at offset 0 of command table
        let fis = ct;
        ptr::write_bytes(fis, 0, 20);
        *fis.add(0) = FIS_TYPE_REG_H2D;
        *fis.add(1) = 0x80; // Command flag
        *fis.add(2) = command;
        *fis.add(3) = 0;    // Feature low
        *fis.add(4) = (lba & 0xFF) as u8;
        *fis.add(5) = ((lba >> 8) & 0xFF) as u8;
        *fis.add(6) = ((lba >> 16) & 0xFF) as u8;
        *fis.add(7) = 1 << 6; // LBA mode
        *fis.add(8) = ((lba >> 24) & 0xFF) as u8;
        *fis.add(9) = ((lba >> 32) & 0xFF) as u8;
        *fis.add(10) = ((lba >> 40) & 0xFF) as u8;
        // count = 1 at offset 12 (little-endian u16)
        *(fis.add(12) as *mut u16) = 1;

        // Issue command slot 0
        port_write(port, 0x38, 1); // CI = bit 0
    }

    port_wait(port, 500000)
}

/// Initialize AHCI controller. Returns the first active port index, or -1.
pub fn ahci_init() -> i32 {
    let dev = match pci_find_device(0x01, 0x06) {
        Some(d) => d,
        None => return -1,
    };
    pci_enable_device(&dev);

    let bar5 = pci_read_bar(&dev, 5);
    let base = (bar5 & !0xF) as *mut u8;
    if base.is_null() {
        return -1;
    }
    unsafe { core::ptr::write_volatile(&raw mut HBA, base); }

    // Enable AHCI mode
    hba_write(0x04, hba_read(0x04) | HBA_GHC_AE);

    let pi = hba_read(0x0C); // Ports Implemented

    let mut first_active: i32 = -1;

    for i in 0..AHCI_MAX_PORTS {
        if pi & (1 << i) == 0 { continue; }

        let ssts = port_read(i, 0x28);
        if ssts & 0xF != SSTS_DET_PRESENT { continue; }

        init_port(i);

        // Issue IDENTIFY
        unsafe { ptr::write_bytes(sbuf(), 0, SECTOR_SIZE); }
        if issue_cmd(i, ATA_CMD_IDENTIFY, 0, false) {
            let id = sbuf() as *const u16;
            unsafe {
                let mut sectors = ptr::read(id.add(60)) as u64
                    | (ptr::read(id.add(61)) as u64) << 16;
                // Check 48-bit LBA support (word 83, bit 10)
                if ptr::read(id.add(83)) & (1 << 10) != 0 {
                    sectors = ptr::read(id.add(100)) as u64
                        | (ptr::read(id.add(101)) as u64) << 16
                        | (ptr::read(id.add(102)) as u64) << 32
                        | (ptr::read(id.add(103)) as u64) << 48;
                }
                core::ptr::write_volatile((&raw mut PORT_SECTORS).cast::<u64>().add(i), sectors);
                core::ptr::write_volatile((&raw mut PORT_ACTIVE).cast::<bool>().add(i), true);
            }
            if first_active < 0 {
                first_active = i as i32;
            }
        }
    }

    unsafe { core::ptr::write_volatile(&raw mut ACTIVE_PORT, first_active); }
    first_active
}

pub fn ahci_read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let port = unsafe { core::ptr::read_volatile(&raw const ACTIVE_PORT) };
    if port < 0 { return Err("NO DISK"); }
    let p = port as usize;

    unsafe { ptr::write_bytes(sbuf(), 0, SECTOR_SIZE); }
    if !issue_cmd(p, ATA_CMD_READ_DMA_EX, lba as u64, false) {
        return Err("DISK ERROR");
    }
    unsafe { ptr::copy_nonoverlapping(sbuf(), buf.as_mut_ptr(), SECTOR_SIZE); }
    Ok(())
}

pub fn ahci_write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    let port = unsafe { core::ptr::read_volatile(&raw const ACTIVE_PORT) };
    if port < 0 { return Err("NO DISK"); }
    let p = port as usize;

    unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), sbuf(), SECTOR_SIZE); }
    if !issue_cmd(p, ATA_CMD_WRITE_DMA_EX, lba as u64, true) {
        return Err("DISK ERROR");
    }
    Ok(())
}

pub fn ahci_get_total_sectors() -> u32 {
    let port = unsafe { core::ptr::read_volatile(&raw const ACTIVE_PORT) };
    if port < 0 { return 0; }
    unsafe { core::ptr::read_volatile((&raw const PORT_SECTORS).cast::<u64>().add(port as usize)) as u32 }
}
