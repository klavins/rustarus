// gpu.rs - GPU driver trait and probe framework
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

use crate::cell::StaticCell;
use crate::console::Console;
use crate::pci::pci_read;

/// Active GPU driver — enum dispatch avoids dynamic trait objects.
/// Each step that adds a GPU driver adds a variant here.
pub enum ActiveGpu {
    None,
    // Step 7 will add: Bga(BgaDriver),
    // Step 7 will add: Vmware(VmwareDriver),
}

impl ActiveGpu {
    pub fn is_active(&self) -> bool {
        !matches!(self, ActiveGpu::None)
    }

    pub fn can_flip(&self) -> bool {
        match self {
            ActiveGpu::None => false,
        }
    }

    pub fn page_addr(&self, _page: u8) -> *mut u8 {
        match self {
            ActiveGpu::None => core::ptr::null_mut(),
        }
    }

    pub fn set_page(&mut self, _page: u8) {
        match self {
            ActiveGpu::None => {}
        }
    }

    pub fn update(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {
        match self {
            ActiveGpu::None => {}
        }
    }

    pub fn pitch(&self) -> u32 {
        match self {
            ActiveGpu::None => 0,
        }
    }

    pub fn framebuffer(&self) -> *mut u8 {
        match self {
            ActiveGpu::None => core::ptr::null_mut(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ActiveGpu::None => "none",
        }
    }
}

pub static GPU: StaticCell<ActiveGpu> = StaticCell::new(ActiveGpu::None);

/// Probe registered GPU drivers and activate the first match.
/// Called during boot after console init.
fn print_hex16(con: &mut Console, val: u16) {
    let hex = b"0123456789ABCDEF";
    con.putchar(hex[((val >> 12) & 0xF) as usize]);
    con.putchar(hex[((val >> 8) & 0xF) as usize]);
    con.putchar(hex[((val >> 4) & 0xF) as usize]);
    con.putchar(hex[(val & 0xF) as usize]);
}

fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x00, 0x00) => "Non-VGA unclassified",
        (0x00, 0x01) => "VGA compatible",
        (0x01, 0x00) => "SCSI controller",
        (0x01, 0x01) => "IDE controller",
        (0x01, 0x02) => "Floppy controller",
        (0x01, 0x06) => "SATA controller",
        (0x01, 0x08) => "NVMe controller",
        (0x01, _)    => "Storage controller",
        (0x02, 0x00) => "Ethernet controller",
        (0x02, _)    => "Network controller",
        (0x03, 0x00) => "VGA controller",
        (0x03, 0x80) => "Display controller",
        (0x03, _)    => "Display controller",
        (0x04, 0x00) => "Video device",
        (0x04, 0x01) => "Audio device",
        (0x04, _)    => "Multimedia device",
        (0x05, _)    => "Memory controller",
        (0x06, 0x00) => "Host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x06, 0x04) => "PCI bridge",
        (0x06, 0x80) => "Other bridge",
        (0x06, _)    => "Bridge device",
        (0x07, _)    => "Communication controller",
        (0x08, _)    => "System peripheral",
        (0x09, _)    => "Input device",
        (0x0C, 0x03) => "USB controller",
        (0x0C, _)    => "Serial bus controller",
        (0x0D, _)    => "Wireless controller",
        _ => "Unknown",
    }
}

fn pci_scan(con: &mut Console) {
    con.print(" PCI devices:\n");
    crate::pci::pci_enumerate(|dev| {
        let id = pci_read(dev, 0x00);
        let vendor = (id & 0xFFFF) as u16;
        let device = ((id >> 16) & 0xFFFF) as u16;
        let class_reg = pci_read(dev, 0x08);
        let class = ((class_reg >> 24) & 0xFF) as u8;
        let subclass = ((class_reg >> 16) & 0xFF) as u8;

        con.print("  ");
        print_hex16(con, vendor);
        con.print(":");
        print_hex16(con, device);
        con.print(" ");
        con.print(class_name(class, subclass));
        con.putchar(b'\n');
    });
}

pub fn gpu_init(con: &mut Console) {
    pci_scan(con);

    let gpu = unsafe { GPU.get() };
    if gpu.is_active() {
        con.print(" Display: GOP -> ");
        con.print(gpu.name());
        con.putchar(b'\n');
    } else {
        con.print(" Display: GOP\n");
    }
}

/// Notify the active GPU of a framebuffer region change.
#[inline]
pub fn gpu_update(x: u32, y: u32, w: u32, h: u32) {
    let gpu = unsafe { GPU.get() };
    gpu.update(x, y, w, h);
}
