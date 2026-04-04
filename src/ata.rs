// ata.rs - ATA PIO disk driver
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

use crate::io::{outb, inb, outw, inw};

const ATA_DATA: u16 = 0x1F0;
#[allow(dead_code)]
const ATA_ERROR: u16 = 0x1F1;
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

pub const SECTOR_SIZE: usize = 512;

fn wait_ready() -> Result<(), &'static str> {
    for _ in 0..TIMEOUT {
        let status = unsafe { inb(ATA_STATUS) };
        if status == 0xFF {
            return Err("NO DISK");
        }
        if status & ATA_SR_ERR != 0 {
            return Err("DISK ERROR");
        }
        if status & ATA_SR_BSY == 0 {
            return Ok(());
        }
    }
    Err("DISK TIMEOUT")
}

fn wait_drq() -> Result<(), &'static str> {
    for _ in 0..TIMEOUT {
        let status = unsafe { inb(ATA_STATUS) };
        if status & ATA_SR_ERR != 0 {
            return Err("DISK ERROR");
        }
        if status & ATA_SR_DRQ != 0 {
            return Ok(());
        }
    }
    Err("DISK TIMEOUT")
}

fn select_lba(lba: u32) {
    unsafe {
        outb(ATA_DRIVE, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(ATA_COUNT, 1);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
    }
}

pub fn ata_read_sector(lba: u32, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    wait_ready()?;
    select_lba(lba);
    unsafe { outb(ATA_CMD, ATA_CMD_READ); }
    wait_drq()?;

    let words = buf.as_mut_ptr() as *mut u16;
    for i in 0..256 {
        unsafe {
            let w = inw(ATA_DATA);
            core::ptr::write(words.add(i), w);
        }
    }
    Ok(())
}

pub fn ata_write_sector(lba: u32, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    wait_ready()?;
    select_lba(lba);
    unsafe { outb(ATA_CMD, ATA_CMD_WRITE); }
    wait_drq()?;

    let words = buf.as_ptr() as *const u16;
    for i in 0..256 {
        unsafe {
            let w = core::ptr::read(words.add(i));
            outw(ATA_DATA, w);
        }
    }
    wait_ready()?;
    Ok(())
}

pub fn ata_get_total_sectors() -> u32 {
    if wait_ready().is_err() {
        return 0;
    }
    unsafe {
        outb(ATA_DRIVE, 0xE0);
        outb(ATA_CMD, ATA_CMD_IDENTIFY);
    }
    if wait_drq().is_err() {
        return 0;
    }

    let mut ident = [0u16; 256];
    for i in 0..256 {
        ident[i] = unsafe { inw(ATA_DATA) };
    }

    // Words 60-61: total addressable sectors (LBA28)
    (ident[61] as u32) << 16 | ident[60] as u32
}
