// vmware.rs - VMware SVGA II driver
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

use crate::os::io::{outl, inl};
use crate::os::pci::{pci_find_vendor, pci_read_bar, pci_enable_device};
use core::ptr;

const VMWARE_PCI_VENDOR: u16 = 0x15AD;
const VMWARE_PCI_DEVICE: u16 = 0x0405;
const SVGA_ID_2: u32 = 0x90000002;

// Register indices
const SVGA_REG_ID: u32 = 0;
const SVGA_REG_ENABLE: u32 = 1;
const SVGA_REG_WIDTH: u32 = 2;
const SVGA_REG_HEIGHT: u32 = 3;
const SVGA_REG_BITS_PER_PIXEL: u32 = 7;
const SVGA_REG_BYTES_PER_LINE: u32 = 12;
const SVGA_REG_FB_OFFSET: u32 = 14;
const SVGA_REG_VRAM_SIZE: u32 = 15;
const SVGA_REG_MEM_SIZE: u32 = 19;
const SVGA_REG_CONFIG_DONE: u32 = 20;
const SVGA_REG_SYNC: u32 = 21;
const SVGA_REG_BUSY: u32 = 22;

// FIFO header offsets (dword indices)
const SVGA_FIFO_MIN: usize = 0;
const SVGA_FIFO_MAX: usize = 1;
const SVGA_FIFO_NEXT_CMD: usize = 2;
const SVGA_FIFO_STOP: usize = 3;

const SVGA_CMD_UPDATE: u32 = 1;

pub struct VmwareDriver {
    io_base: u16,
    fb_base: *mut u8,
    fifo: *mut u32,
    fb_offset: u32,
    width: u32,
    height: u32,
    pitch: u32,
    vram_size: u32,
    flip: bool,
}

unsafe impl Send for VmwareDriver {}

impl VmwareDriver {
    fn svga_write(&self, reg: u32, val: u32) {
        unsafe {
            outl(self.io_base, reg);
            outl(self.io_base + 1, val);
        }
    }

    fn svga_read(&self, reg: u32) -> u32 {
        unsafe {
            outl(self.io_base, reg);
            inl(self.io_base + 1)
        }
    }

    fn svga_sync(&self) {
        self.svga_write(SVGA_REG_SYNC, 1);
        while self.svga_read(SVGA_REG_BUSY) != 0 {}
    }

    fn fifo_write_cmd(&mut self, cmd: u32, args: &[u32]) {
        unsafe {
            let fifo = self.fifo;
            let min = ptr::read_volatile(fifo.add(SVGA_FIFO_MIN));
            let max = ptr::read_volatile(fifo.add(SVGA_FIFO_MAX));
            let mut next = ptr::read_volatile(fifo.add(SVGA_FIFO_NEXT_CMD));

            // Ensure space: need (1 + args.len()) dwords. Sync if FIFO is too full.
            let needed = (1 + args.len() as u32) * 4;
            let stop = ptr::read_volatile(fifo.add(SVGA_FIFO_STOP));
            let avail = if next >= stop {
                (max - min) - (next - stop)
            } else {
                stop - next
            };
            if avail < needed {
                self.svga_sync();
            }

            // Reload next after potential sync
            next = ptr::read_volatile(fifo.add(SVGA_FIFO_NEXT_CMD));

            ptr::write_volatile(fifo.add((next / 4) as usize), cmd);
            next += 4;
            if next >= max { next = min; }

            for &arg in args {
                ptr::write_volatile(fifo.add((next / 4) as usize), arg);
                next += 4;
                if next >= max { next = min; }
            }

            ptr::write_volatile(fifo.add(SVGA_FIFO_NEXT_CMD), next);
        }
    }

    pub fn detect() -> Option<Self> {
        let dev = pci_find_vendor(VMWARE_PCI_VENDOR, VMWARE_PCI_DEVICE)?;
        pci_enable_device(&dev);

        let bar0 = pci_read_bar(&dev, 0);
        let io_base = (bar0 & 0xFFFFFFFC) as u16;

        let mut drv = Self {
            io_base,
            fb_base: core::ptr::null_mut(),
            fifo: core::ptr::null_mut(),
            fb_offset: 0,
            width: 0,
            height: 0,
            pitch: 0,
            vram_size: 0,
            flip: false,
        };

        // Negotiate version
        drv.svga_write(SVGA_REG_ID, SVGA_ID_2);
        let id = drv.svga_read(SVGA_REG_ID);
        if id != SVGA_ID_2 {
            return None;
        }

        let bar1 = pci_read_bar(&dev, 1);
        drv.fb_base = (bar1 & 0xFFFFFFF0) as *mut u8;
        drv.vram_size = drv.svga_read(SVGA_REG_VRAM_SIZE);

        let bar2 = pci_read_bar(&dev, 2);
        drv.fifo = (bar2 & 0xFFFFFFF0) as *mut u32;

        Some(drv)
    }

    pub fn init(&mut self, width: u32, height: u32) {
        self.svga_write(SVGA_REG_WIDTH, width);
        self.svga_write(SVGA_REG_HEIGHT, height);
        self.svga_write(SVGA_REG_BITS_PER_PIXEL, 32);
        self.svga_write(SVGA_REG_ENABLE, 1);

        self.width = self.svga_read(SVGA_REG_WIDTH);
        self.height = self.svga_read(SVGA_REG_HEIGHT);
        self.pitch = self.svga_read(SVGA_REG_BYTES_PER_LINE);
        self.fb_offset = self.svga_read(SVGA_REG_FB_OFFSET);

        let page_size = self.pitch * self.height;
        self.flip = self.vram_size >= self.fb_offset + page_size * 2;

        // Initialize FIFO
        let fifo_size = self.svga_read(SVGA_REG_MEM_SIZE);
        unsafe {
            ptr::write_volatile(self.fifo.add(SVGA_FIFO_MIN), 16);
            ptr::write_volatile(self.fifo.add(SVGA_FIFO_MAX), fifo_size);
            ptr::write_volatile(self.fifo.add(SVGA_FIFO_NEXT_CMD), 16);
            ptr::write_volatile(self.fifo.add(SVGA_FIFO_STOP), 16);
        }
        self.svga_write(SVGA_REG_CONFIG_DONE, 1);
    }

    pub fn name(&self) -> &'static str { "VMSVGA" }

    pub fn framebuffer(&self) -> *mut u8 {
        unsafe { self.fb_base.add(self.fb_offset as usize) }
    }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn pitch(&self) -> u32 { self.pitch }

    pub fn can_flip(&self) -> bool { false }

    pub fn page_addr(&self, page: u8) -> *mut u8 {
        let page_size = self.pitch * self.height;
        let offset = self.fb_offset + if page == 0 { 0 } else { page_size };
        unsafe { self.fb_base.add(offset as usize) }
    }

    pub fn set_page(&mut self, page: u8) {
        let page_size = self.pitch * self.height;
        let offset = self.fb_offset + if page == 0 { 0 } else { page_size };
        self.svga_write(SVGA_REG_FB_OFFSET, offset);
        self.fifo_write_cmd(SVGA_CMD_UPDATE, &[0, 0, self.width, self.height]);
        self.svga_sync();
    }

    pub fn update(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.fifo_write_cmd(SVGA_CMD_UPDATE, &[x, y, w, h]);
        self.svga_sync();
    }

    pub fn wait_vsync(&self) {
        // VMware SVGA uses FIFO commands, no vsync available
    }
}
