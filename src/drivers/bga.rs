// bga.rs - Bochs Graphics Adapter (BGA) driver
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

use crate::os::io::{outw, inw};
use crate::os::pci::{PciDevice, pci_find_vendor, pci_read_bar, pci_enable_device};

const VBE_INDEX: u16 = 0x01CE;
const VBE_DATA: u16 = 0x01CF;

const VBE_ID: u16 = 0x00;
const VBE_XRES: u16 = 0x01;
const VBE_YRES: u16 = 0x02;
const VBE_BPP: u16 = 0x03;
const VBE_ENABLE: u16 = 0x04;
const VBE_VIRT_WIDTH: u16 = 0x06;
const VBE_VIRT_HEIGHT: u16 = 0x07;
const VBE_X_OFFSET: u16 = 0x08;
const VBE_Y_OFFSET: u16 = 0x09;

const VBE_DISABLED: u16 = 0x00;
const VBE_ENABLED: u16 = 0x01;
const VBE_LFB_ENABLED: u16 = 0x40;
const VBE_NOCLEARMEM: u16 = 0x80;

const BGA_PCI_VENDOR: u16 = 0x1234;
const BGA_PCI_DEVICE: u16 = 0x1111;

fn vbe_write(reg: u16, val: u16) {
    unsafe {
        outw(VBE_INDEX, reg);
        outw(VBE_DATA, val);
    }
}

fn vbe_read(reg: u16) -> u16 {
    unsafe {
        outw(VBE_INDEX, reg);
        inw(VBE_DATA)
    }
}

pub struct BgaDriver {
    dev: PciDevice,
    fb_base: *mut u8,
    width: u32,
    height: u32,
    pitch: u32,
    flip: bool,
}

unsafe impl Send for BgaDriver {}

impl BgaDriver {
    pub fn detect() -> Option<Self> {
        let dev = pci_find_vendor(BGA_PCI_VENDOR, BGA_PCI_DEVICE)?;
        let id = vbe_read(VBE_ID);
        if id < 0xB0C0 {
            return None;
        }
        Some(Self {
            dev,
            fb_base: core::ptr::null_mut(),
            width: 0,
            height: 0,
            pitch: 0,
            flip: false,
        })
    }

    pub fn init(&mut self, width: u32, height: u32) {
        pci_enable_device(&self.dev);

        let bar0 = pci_read_bar(&self.dev, 0);
        self.fb_base = (bar0 & 0xFFFFFFF0) as *mut u8;

        vbe_write(VBE_ENABLE, VBE_DISABLED);
        vbe_write(VBE_XRES, width as u16);
        vbe_write(VBE_YRES, height as u16);
        vbe_write(VBE_BPP, 32);
        vbe_write(VBE_VIRT_WIDTH, width as u16);
        vbe_write(VBE_VIRT_HEIGHT, (height * 2) as u16);
        vbe_write(VBE_X_OFFSET, 0);
        vbe_write(VBE_Y_OFFSET, 0);
        vbe_write(VBE_ENABLE, VBE_ENABLED | VBE_LFB_ENABLED | VBE_NOCLEARMEM);

        self.width = width;
        self.height = height;
        self.pitch = width * 4;

        let actual_vh = vbe_read(VBE_VIRT_HEIGHT) as u32;
        self.flip = actual_vh >= height * 2;
    }

    pub fn name(&self) -> &'static str { "BGA" }

    pub fn framebuffer(&self) -> *mut u8 { self.fb_base }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn pitch(&self) -> u32 { self.pitch }

    pub fn can_flip(&self) -> bool { self.flip }

    pub fn page_addr(&self, page: u8) -> *mut u8 {
        if page == 0 {
            self.fb_base
        } else {
            unsafe { self.fb_base.add((self.height * self.pitch) as usize) }
        }
    }

    pub fn set_page(&mut self, page: u8) {
        let y_off = if page == 0 { 0 } else { self.height as u16 };
        vbe_write(VBE_Y_OFFSET, y_off);
    }

    pub fn update(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {
        // BGA doesn't need update notifications
    }
}
