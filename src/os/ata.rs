// ata.rs - ATA disk driver with AHCI and PIO backends
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

use crate::os::io::{outb, inb, outw, inw};

pub const SECTOR_SIZE: usize = 512;

static mut USE_AHCI: bool = false;

/// Initialize disk — try AHCI first, fall back to PIO.
pub fn ata_init() {
    if crate::os::ahci::ahci_init() >= 0 {
        unsafe { USE_AHCI = true; }
    }
}

pub fn ata_read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    if unsafe { USE_AHCI } {
        crate::os::ahci::ahci_read_sector(lba, buf)
    } else {
        pio_read_sector(lba, buf)
    }
}

pub fn ata_write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    if unsafe { USE_AHCI } {
        crate::os::ahci::ahci_write_sector(lba, buf)
    } else {
        pio_write_sector(lba, buf)
    }
}

pub fn ata_get_total_sectors() -> u32 {
    if unsafe { USE_AHCI } {
        crate::os::ahci::ahci_get_total_sectors()
    } else {
        pio_get_total_sectors()
    }
}

// --- Legacy PIO backend (ports 0x1F0-0x1F7) ---

const ATA_DATA: u16 = 0x1F0;
const ATA_COUNT: u16 = 0x1F2;
const ATA_LBA_LO: u16 = 0x1F3;
const ATA_LBA_MID: u16 = 0x1F4;
const ATA_LBA_HI: u16 = 0x1F5;
const ATA_DRIVE: u16 = 0x1F6;
const ATA_CMD: u16 = 0x1F7;
const ATA_STATUS: u16 = 0x1F7;

const ATA_CMD_READ: u8 = 0x20;
const ATA_CMD_WRITE: u8 = 0x30;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

const ATA_SR_BSY: u8 = 0x80;
const ATA_SR_DRQ: u8 = 0x08;
const ATA_SR_ERR: u8 = 0x01;

const TIMEOUT: u32 = 1_000_000;

fn pio_wait_ready() -> Result<(), &'static str> {
    for _ in 0..TIMEOUT {
        let status = unsafe { inb(ATA_STATUS) };
        if status == 0xFF { return Err("NO DISK"); }
        if status & ATA_SR_ERR != 0 { return Err("DISK ERROR"); }
        if status & ATA_SR_BSY == 0 { return Ok(()); }
    }
    Err("DISK TIMEOUT")
}

fn pio_wait_drq() -> Result<(), &'static str> {
    for _ in 0..TIMEOUT {
        let status = unsafe { inb(ATA_STATUS) };
        if status & ATA_SR_ERR != 0 { return Err("DISK ERROR"); }
        if status & ATA_SR_DRQ != 0 { return Ok(()); }
    }
    Err("DISK TIMEOUT")
}

fn pio_select_lba(lba: u32) {
    unsafe {
        outb(ATA_DRIVE, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(ATA_COUNT, 1);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
    }
}

fn pio_read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    pio_wait_ready()?;
    pio_select_lba(lba);
    unsafe { outb(ATA_CMD, ATA_CMD_READ); }
    pio_wait_drq()?;
    let words = buf.as_mut_ptr() as *mut u16;
    for i in 0..256 {
        unsafe { core::ptr::write(words.add(i), inw(ATA_DATA)); }
    }
    Ok(())
}

fn pio_write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    pio_wait_ready()?;
    pio_select_lba(lba);
    unsafe { outb(ATA_CMD, ATA_CMD_WRITE); }
    pio_wait_drq()?;
    let words = buf.as_ptr() as *const u16;
    for i in 0..256 {
        unsafe { outw(ATA_DATA, core::ptr::read(words.add(i))); }
    }
    pio_wait_ready()?;
    Ok(())
}

fn pio_get_total_sectors() -> u32 {
    if pio_wait_ready().is_err() { return 0; }
    unsafe {
        outb(ATA_DRIVE, 0xE0);
        outb(ATA_CMD, ATA_CMD_IDENTIFY);
    }
    if pio_wait_drq().is_err() { return 0; }
    let mut ident = [0u16; 256];
    for i in 0..256 {
        ident[i] = unsafe { inw(ATA_DATA) };
    }
    (ident[61] as u32) << 16 | ident[60] as u32
}
