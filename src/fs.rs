// fs.rs - Simple flat filesystem compatible with icarus disk images
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

use crate::ata::{ata_read_sector, ata_write_sector, ata_get_total_sectors, SECTOR_SIZE};
use crate::console::Console;
use crate::basic::value::print_f64;

const FS_MAGIC: u32 = 0x49434152; // "ICAR"
const FS_MAX_FILES: usize = 32;
const FS_NAME_LEN: usize = 32;
const FS_DATA_START: u32 = 33; // 1 header + 32 directory sectors

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
}

fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    let bytes = val.to_le_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

/// Read and validate the filesystem header. Returns file_count.
fn read_validated_header() -> Result<u32, &'static str> {
    let mut sector = [0u8; SECTOR_SIZE];
    ata_read_sector(0, &mut sector)?;
    let magic = read_u32(&sector, 0);
    if magic != FS_MAGIC {
        return Err("DISK NOT FORMATTED");
    }
    Ok(read_u32(&sector, 4))
}

fn write_header(file_count: u32) -> Result<(), &'static str> {
    let mut sector = [0u8; SECTOR_SIZE];
    write_u32(&mut sector, 0, FS_MAGIC);
    write_u32(&mut sector, 4, file_count);
    ata_write_sector(0, &sector)
}

struct DirEntry {
    name: [u8; FS_NAME_LEN],
    name_len: usize,
    start_sector: u32,
    size_bytes: u32,
}

fn read_entry(idx: usize) -> Result<DirEntry, &'static str> {
    let mut sector = [0u8; SECTOR_SIZE];
    ata_read_sector(1 + idx as u32, &mut sector)?;

    let mut name = [0u8; FS_NAME_LEN];
    let mut name_len = 0;
    for i in 0..FS_NAME_LEN {
        if sector[i] == 0 {
            break;
        }
        name[i] = sector[i];
        name_len = i + 1;
    }

    let start_sector = read_u32(&sector, FS_NAME_LEN);
    let size_bytes = read_u32(&sector, FS_NAME_LEN + 4);

    Ok(DirEntry { name, name_len, start_sector, size_bytes })
}

fn write_entry(idx: usize, name: &[u8], start_sector: u32, size_bytes: u32) -> Result<(), &'static str> {
    let mut sector = [0u8; SECTOR_SIZE];
    let copy_len = name.len().min(FS_NAME_LEN - 1);
    sector[..copy_len].copy_from_slice(&name[..copy_len]);
    write_u32(&mut sector, FS_NAME_LEN, start_sector);
    write_u32(&mut sector, FS_NAME_LEN + 4, size_bytes);
    ata_write_sector(1 + idx as u32, &sector)
}

fn names_match(a: &[u8], a_len: usize, b: &[u8], b_len: usize) -> bool {
    if a_len != b_len {
        return false;
    }
    for i in 0..a_len {
        if a[i].to_ascii_uppercase() != b[i].to_ascii_uppercase() {
            return false;
        }
    }
    true
}

fn find_free_sector(file_count: u32) -> Result<u32, &'static str> {
    let mut next_free = FS_DATA_START;
    for i in 0..file_count as usize {
        let entry = read_entry(i)?;
        let end = entry.start_sector + (entry.size_bytes + SECTOR_SIZE as u32 - 1) / SECTOR_SIZE as u32;
        if end > next_free {
            next_free = end;
        }
    }
    Ok(next_free)
}

pub fn fs_format() -> Result<(), &'static str> {
    write_header(0)?;
    let empty = [0u8; SECTOR_SIZE];
    for i in 0..FS_MAX_FILES {
        ata_write_sector(1 + i as u32, &empty)?;
    }
    Ok(())
}

pub fn fs_save(name: &[u8], data: &[u8]) -> Result<(), &'static str> {
    let mut file_count = read_validated_header()?;

    // Delete existing file with same name (inline to avoid redundant I/O)
    let name_len = name.len();
    for i in 0..file_count as usize {
        let entry = read_entry(i)?;
        if names_match(&entry.name, entry.name_len, name, name_len) {
            for j in i..file_count as usize - 1 {
                let next = read_entry(j + 1)?;
                write_entry(j, &next.name[..next.name_len], next.start_sector, next.size_bytes)?;
            }
            file_count -= 1;
            write_header(file_count)?;
            break;
        }
    }

    if file_count as usize >= FS_MAX_FILES {
        return Err("DIRECTORY FULL");
    }

    let start = find_free_sector(file_count)?;
    let sectors_needed = (data.len() as u32 + SECTOR_SIZE as u32 - 1) / SECTOR_SIZE as u32;

    // Write data sectors
    for s in 0..sectors_needed {
        let mut sector = [0u8; SECTOR_SIZE];
        let offset = (s as usize) * SECTOR_SIZE;
        let remaining = data.len() - offset;
        let copy_len = remaining.min(SECTOR_SIZE);
        sector[..copy_len].copy_from_slice(&data[offset..offset + copy_len]);
        ata_write_sector(start + s, &sector)?;
    }

    // Write directory entry
    write_entry(file_count as usize, name, start, data.len() as u32)?;
    write_header(file_count + 1)?;
    Ok(())
}

pub fn fs_load(name: &[u8], buf: &mut [u8], max: usize) -> Result<usize, &'static str> {
    let file_count = read_validated_header()?;

    let name_len = name.len();
    for i in 0..file_count as usize {
        let entry = read_entry(i)?;
        if names_match(&entry.name, entry.name_len, name, name_len) {
            let size = (entry.size_bytes as usize).min(max);
            let sectors = (size + SECTOR_SIZE - 1) / SECTOR_SIZE;
            let mut sector = [0u8; SECTOR_SIZE];
            let mut pos = 0;
            for s in 0..sectors {
                ata_read_sector(entry.start_sector + s as u32, &mut sector)?;
                let remaining = size - pos;
                let copy_len = remaining.min(SECTOR_SIZE);
                buf[pos..pos + copy_len].copy_from_slice(&sector[..copy_len]);
                pos += copy_len;
            }
            return Ok(size);
        }
    }
    Err("FILE NOT FOUND")
}

pub fn fs_delete(name: &[u8]) -> Result<(), &'static str> {
    let file_count = read_validated_header()?;

    let name_len = name.len();
    for i in 0..file_count as usize {
        let entry = read_entry(i)?;
        if names_match(&entry.name, entry.name_len, name, name_len) {
            // Shift remaining entries down
            for j in i..file_count as usize - 1 {
                let next = read_entry(j + 1)?;
                write_entry(j, &next.name[..next.name_len], next.start_sector, next.size_bytes)?;
            }
            write_header(file_count - 1)?;
            return Ok(());
        }
    }
    Err("FILE NOT FOUND")
}

pub fn fs_list(con: &mut Console) {
    let file_count = match read_validated_header() {
        Ok(c) => c,
        Err(_) => { con.print(" Disk not formatted\n"); return; }
    };

    if file_count == 0 {
        con.print(" (no files)\n");
    } else {
        for i in 0..file_count as usize {
            if let Ok(entry) = read_entry(i) {
                con.print("  ");
                for j in 0..entry.name_len {
                    con.putchar(entry.name[j]);
                }
                // Pad to 16 chars
                for _ in entry.name_len..16 {
                    con.putchar(b' ');
                }
                print_f64(con, entry.size_bytes as f64);
                con.print(" bytes\n");
            }
        }
    }

    // Free space
    let total_sectors = ata_get_total_sectors();
    if total_sectors > 0 {
        let next_free = find_free_sector(file_count).unwrap_or(FS_DATA_START);
        let free_bytes = (total_sectors - next_free) as f64 * SECTOR_SIZE as f64;
        con.print("\n  ");
        print_f64(con, file_count as f64);
        con.print(" files, ");
        print_f64(con, free_bytes);
        con.print(" bytes free\n");
    }
}
