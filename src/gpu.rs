// gpu.rs - GPU driver probe framework
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

use crate::bga::BgaDriver;
use crate::cell::StaticCell;
use crate::console::Console;
use crate::nvidia::NvidiaDriver;
use crate::pci::pci_read;
use crate::vmware::VmwareDriver;

/// Active GPU driver — enum dispatch avoids dynamic trait objects.
pub enum ActiveGpu {
    None,
    Nvidia(NvidiaDriver),
    Bga(BgaDriver),
    Vmware(VmwareDriver),
}

macro_rules! gpu_dispatch {
    ($self:expr, $default:expr, |$d:ident| $body:expr) => {
        match $self {
            ActiveGpu::None => $default,
            ActiveGpu::Nvidia($d) => $body,
            ActiveGpu::Bga($d) => $body,
            ActiveGpu::Vmware($d) => $body,
        }
    };
}

impl ActiveGpu {
    pub fn is_active(&self) -> bool { !matches!(self, ActiveGpu::None) }
    pub fn can_flip(&self) -> bool { gpu_dispatch!(self, false, |d| d.can_flip()) }
    pub fn page_addr(&self, page: u8) -> *mut u8 { gpu_dispatch!(self, core::ptr::null_mut(), |d| d.page_addr(page)) }
    pub fn set_page(&mut self, page: u8) { gpu_dispatch!(self, {}, |d| d.set_page(page)) }
    pub fn update(&mut self, x: u32, y: u32, w: u32, h: u32) { gpu_dispatch!(self, {}, |d| d.update(x, y, w, h)) }
    pub fn pitch(&self) -> u32 { gpu_dispatch!(self, 0, |d| d.pitch()) }
    pub fn framebuffer(&self) -> *mut u8 { gpu_dispatch!(self, core::ptr::null_mut(), |d| d.framebuffer()) }
    pub fn name(&self) -> &'static str { gpu_dispatch!(self, "none", |d| d.name()) }
}

pub static GPU: StaticCell<ActiveGpu> = StaticCell::new(ActiveGpu::None);

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

/// Try to activate a GPU driver. Returns true if successful.
fn try_activate(
    gpu: &mut ActiveGpu,
    active: ActiveGpu,
    con: &mut Console,
) -> bool {
    let fb = gpu_dispatch!(&active, core::ptr::null_mut(), |d| d.framebuffer());
    if fb.is_null() {
        return false;
    }
    let name = gpu_dispatch!(&active, "", |d| d.name());
    let flip = gpu_dispatch!(&active, false, |d| d.can_flip());

    // Switch console and graphics to the GPU's framebuffer
    let old_fb = con.fb_addr();
    if fb != old_fb {
        con.set_fb_addr(fb);
        let gfx = unsafe { crate::graphics::GRAPHICS.get() };
        gfx.set_fb_addr(fb);
        // Flush current shadow buffer content to the new framebuffer
        con.clear();
    }

    con.print(" Display: GOP -> ");
    con.print(name);
    if flip { con.print(" (page flip)"); }
    con.putchar(b'\n');
    *gpu = active;
    true
}

/// Probe GPU drivers and activate the first match.
pub fn gpu_init(con: &mut Console, fb: *mut u8, width: u32, height: u32, pitch: u32) {
    pci_scan(con);

    let gpu = unsafe { GPU.get() };

    // NVIDIA first (highest priority) — needs GOP fb info for WC path
    if let Some(mut drv) = NvidiaDriver::detect(con) {
        drv.init(fb, width, height, pitch, con);
        if try_activate(gpu, ActiveGpu::Nvidia(drv), con) { return; }
    }

    if let Some(mut drv) = BgaDriver::detect() {
        drv.init(width, height);
        if try_activate(gpu, ActiveGpu::Bga(drv), con) { return; }
    }

    if let Some(mut drv) = VmwareDriver::detect() {
        drv.init(width, height);
        if try_activate(gpu, ActiveGpu::Vmware(drv), con) { return; }
    }

    con.print(" Display: GOP\n");
}

/// Notify the active GPU of a framebuffer region change.
#[inline]
pub fn gpu_update(x: u32, y: u32, w: u32, h: u32) {
    let gpu = unsafe { GPU.get() };
    gpu.update(x, y, w, h);
}
