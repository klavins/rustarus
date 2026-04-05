// nvidia.rs - NVIDIA GTX 1650 (TU117) reverse-engineered EVO display driver
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

use crate::console::Console;
use crate::pci::{pci_find_vendor, pci_read_bar, pci_enable_device};
use core::arch::asm;
use core::ptr;

// PCI
const NV_PCI_VENDOR: u16 = 0x10DE;
const KNOWN_DEVICES: &[u16] = &[
    0x1F82, // GTX 1650 (GDDR5, TU117)
    0x1F91, // GTX 1650 Mobile
    0x1F95, // GTX 1650 Ti Mobile
    0x2184, // GTX 1650 (GDDR6, TU116)
    0x2187, // GTX 1650 SUPER (TU116)
];

// MMIO registers
#[allow(dead_code)]
const NV_PMC_BOOT_0: u32 = 0x000000;
const NV_PMC_ENABLE: u32 = 0x000200;
const NV_PDISP_INST_TARGET: u32 = 0x610010;
const NV_PDISP_INST_ADDR: u32 = 0x610014;
const NV_PDISP_MASTER_CTRL: u32 = 0x610078;
const NV_PDISP_OWNERSHIP: u32 = 0x6254E8;

fn nv_pdisp_chan_ctrl(c: u32) -> u32 { 0x6104E0 + c * 4 }
const NV_PDISP_CORE_STAT: u32 = 0x610630;
fn nv_pdisp_win_stat(w: u32) -> u32 { 0x610664 + w * 4 }

fn nv_pdisp_pb_hi(c: u32) -> u32 { 0x610B20 + c * 0x10 }
fn nv_pdisp_pb_lo(c: u32) -> u32 { 0x610B24 + c * 0x10 }
fn nv_pdisp_pb_valid(c: u32) -> u32 { 0x610B28 + c * 0x10 }
fn nv_pdisp_pb_limit(c: u32) -> u32 { 0x610B2C + c * 0x10 }

const NV_PDISP_CORE_PUT: u32 = 0x680000;
fn nv_pdisp_win_put(w: u32) -> u32 { 0x690000 + w * 0x1000 }

fn pb_encode(addr: u32) -> u32 { 0x00000001 | (addr >> 8) }
const PB_SIZE: usize = 4096;

// Push buffer header formats
fn core_hdr(count: u32, method: u32) -> u32 { (count << 18) | (method >> 2) }
fn win_hdr(count: u32, method: u32) -> u32 { (2 << 29) | (count << 18) | method }

// EVO methods — core channel
fn core_head(h: u32, m: u32) -> u32 { 0x2000 + h * 0x400 + m }
fn core_win(w: u32, m: u32) -> u32 { 0x1000 + w * 0x80 + m }
const CORE_UPDATE: u32 = 0x0080;

const HEAD_SET_RASTER_SIZE: u32 = 0x0064;
const HEAD_SET_RASTER_SYNC_END: u32 = 0x0068;
const HEAD_SET_RASTER_BLANK_END: u32 = 0x006C;
const HEAD_SET_RASTER_BLANK_START: u32 = 0x0070;
const HEAD_SET_CONTROL: u32 = 0x0440;

const WIN_SET_CONTROL_CORE: u32 = 0x0000;
const WIN_SET_FORMAT_BOUNDS: u32 = 0x0004;
const WIN_SET_ROTATED_BOUNDS: u32 = 0x0008;
const WIN_SET_USAGE_BOUNDS: u32 = 0x0010;

// EVO methods — window channel
const WIN_UPDATE: u32 = 0x0200;
const WIN_SET_SIZE: u32 = 0x0224;
#[allow(dead_code)]
const WIN_SET_STORAGE: u32 = 0x0228;
#[allow(dead_code)]
const WIN_SET_PARAMS: u32 = 0x022C;
#[allow(dead_code)]
const WIN_SET_PLANAR_STORAGE: u32 = 0x0230;
const WIN_SET_CTX_DMA_ISO: u32 = 0x0240;
const WIN_SET_OFFSET: u32 = 0x0260;
const WIN_SET_POINT_IN: u32 = 0x0290;
const WIN_SET_SIZE_IN: u32 = 0x0298;
const WIN_SET_SIZE_OUT: u32 = 0x02A4;
const WIN_SET_PRESENT_CONTROL: u32 = 0x0308;

// VRAM layout
const PAGE0_VRAM: u32 = 0x000000;
const PAGE1_VRAM: u32 = 0x500000;
const CORE_PB_VRAM: u32 = 0xA00000;
const WIN_PB_VRAM: u32 = 0xA01000;
const DISP_INST_VRAM: u32 = 0xA10000;
const DISP_INST_SIZE: usize = 0x10000;
const DMA_CTX_OFFSET: u32 = 0x2000;
const RAMHT_BITS: u32 = 10;

pub struct NvidiaDriver {
    mmio: *mut u32,
    fb_bar: *mut u8,
    gop_fb: *mut u8,
    gop_width: u32,
    gop_height: u32,
    gop_pitch: u32,
    core_pb: *mut u32,
    core_pb_put: u32,
    win_pb: *mut u32,
    win_pb_put: u32,
    flip_ready: bool,
}

unsafe impl Send for NvidiaDriver {}

impl NvidiaDriver {
    fn nv_rd32(&self, reg: u32) -> u32 {
        unsafe { ptr::read_volatile(self.mmio.add((reg / 4) as usize)) }
    }

    fn nv_wr32(&self, reg: u32, val: u32) {
        unsafe { ptr::write_volatile(self.mmio.add((reg / 4) as usize), val) }
    }

    fn core_push(&mut self, method: u32, data: u32) {
        let off = (self.core_pb_put / 4) as usize;
        unsafe {
            ptr::write_volatile(self.core_pb.add(off), core_hdr(1, method));
            ptr::write_volatile(self.core_pb.add(off + 1), data);
        }
        self.core_pb_put += 8;
    }

    fn core_kick(&self) {
        unsafe { asm!("sfence", options(nostack)); }
        self.nv_wr32(NV_PDISP_CORE_PUT, self.core_pb_put >> 2);
    }

    fn core_wait(&self) {
        for _ in 0..200000u32 {
            if (self.nv_rd32(NV_PDISP_CORE_STAT) & 0x000F0000) == 0x00040000 {
                break;
            }
        }
    }

    fn win_push(&mut self, method: u32, data: u32) {
        let off = (self.win_pb_put / 4) as usize;
        unsafe {
            ptr::write_volatile(self.win_pb.add(off), win_hdr(1, method));
            ptr::write_volatile(self.win_pb.add(off + 1), data);
        }
        self.win_pb_put += 8;
    }

    fn win_push_multi(&mut self, base_method: u32, data: &[u32]) {
        let off = (self.win_pb_put / 4) as usize;
        unsafe {
            ptr::write_volatile(self.win_pb.add(off), win_hdr(data.len() as u32, base_method));
            for (i, &val) in data.iter().enumerate() {
                ptr::write_volatile(self.win_pb.add(off + 1 + i), val);
            }
        }
        self.win_pb_put += 4 + data.len() as u32 * 4;
    }

    fn win_kick(&self) {
        unsafe { asm!("sfence", options(nostack)); }
        self.nv_wr32(nv_pdisp_win_put(0), self.win_pb_put >> 2);
    }

    fn win_wait(&self) {
        for _ in 0..200000u32 {
            if (self.nv_rd32(nv_pdisp_win_stat(0)) & 0x000F0000) == 0x00040000 {
                break;
            }
        }
    }

    fn channel_init(&self, ctrl: u32, pb_vram: u32, put_reg: u32, stat_reg: u32) -> bool {
        self.nv_wr32(nv_pdisp_chan_ctrl(ctrl), 0x00000000);
        for _ in 0..100000u32 {
            if (self.nv_rd32(stat_reg) & 0x000F0000) == 0 { break; }
        }

        self.nv_wr32(nv_pdisp_pb_hi(ctrl), 0x00000000);
        self.nv_wr32(nv_pdisp_pb_lo(ctrl), pb_encode(pb_vram));
        self.nv_wr32(nv_pdisp_pb_valid(ctrl), 0x00000001);
        self.nv_wr32(nv_pdisp_pb_limit(ctrl), 0x00000040);

        self.nv_wr32(nv_pdisp_chan_ctrl(ctrl), 0x00000010);
        for _ in 0..100000u32 {
            if (self.nv_rd32(stat_reg) & 0x000F0000) == 0x00040000 { break; }
        }

        self.nv_wr32(put_reg, 0x00000000);
        self.nv_wr32(nv_pdisp_chan_ctrl(ctrl), 0x00000013);

        for _ in 0..200000u32 {
            let state = (self.nv_rd32(stat_reg) >> 16) & 0xF;
            if state == 0x4 || state == 0xB { return true; }
        }
        false
    }

    fn ramht_hash(chid: u32, handle: u32) -> u32 {
        let mut hash = 0u32;
        let mut h = handle;
        while h != 0 {
            hash ^= h & ((1 << RAMHT_BITS) - 1);
            h >>= RAMHT_BITS;
        }
        hash ^= chid << (RAMHT_BITS - 4);
        hash & ((1 << RAMHT_BITS) - 1)
    }

    fn disp_engine_init(&self) {
        let pmc = self.nv_rd32(NV_PMC_ENABLE);
        if pmc & 0x40000000 == 0 {
            self.nv_wr32(NV_PMC_ENABLE, pmc | 0x40000000);
        }

        // Claim ownership
        if self.nv_rd32(NV_PDISP_OWNERSHIP) & 0x00000002 != 0 {
            self.nv_wr32(NV_PDISP_OWNERSHIP, self.nv_rd32(NV_PDISP_OWNERSHIP) & !1);
            for _ in 0..200000u32 {
                if self.nv_rd32(NV_PDISP_OWNERSHIP) & 0x00000002 == 0 { break; }
            }
        }

        // Capabilities (TU102 offsets)
        self.nv_wr32(0x640008, 0x00000021);

        for i in 0..4u32 {
            let tmp = self.nv_rd32(0x61C000 + i * 0x800);
            if tmp == 0 || (tmp & 0xFFFF0000) == 0xBADF0000 { continue; }
            self.nv_wr32(0x640000, self.nv_rd32(0x640000) | (0x100 << i));
            self.nv_wr32(0x640144 + i * 8, tmp);
        }

        for i in 0..2u32 {
            let tmp = self.nv_rd32(0x616300 + i * 0x800);
            self.nv_wr32(0x640048 + i * 0x20, tmp);
            for j in (0..20u32).step_by(4) {
                let tmp = self.nv_rd32(0x616140 + i * 0x800 + j);
                self.nv_wr32(0x640680 + i * 0x20 + j, tmp);
            }
        }

        for i in 0..8u32 {
            self.nv_wr32(0x640004, self.nv_rd32(0x640004) | (1 << i));
            for j in (0..24u32).step_by(4) {
                let tmp = self.nv_rd32(0x630100 + i * 0x800 + j);
                self.nv_wr32(0x640780 + i * 0x20 + j, tmp);
            }
            self.nv_wr32(0x64000C, self.nv_rd32(0x64000C) | 0x100);
        }

        for i in 0..3u32 {
            let tmp = self.nv_rd32(0x62E000 + i * 4);
            self.nv_wr32(0x640010 + i * 4, tmp);
        }

        self.nv_wr32(NV_PDISP_MASTER_CTRL, self.nv_rd32(NV_PDISP_MASTER_CTRL) | 1);

        // Instance memory with RAMHT and DMA context
        unsafe {
            let inst = self.fb_bar.add(DISP_INST_VRAM as usize);
            ptr::write_bytes(inst, 0, DISP_INST_SIZE);

            // DMA context: VRAM, RDWR, full range
            let dma = inst.add(DMA_CTX_OFFSET as usize) as *mut u32;
            ptr::write_volatile(dma.add(0), 0x000D003D);
            ptr::write_volatile(dma.add(1), 0xFFFFFFFF);
            ptr::write_volatile(dma.add(2), 0x00000000);
            ptr::write_volatile(dma.add(3), 0x00000000);

            // RAMHT entry: handle=1, chid=1
            let slot = Self::ramht_hash(1, 1);
            let entry = inst.add((slot * 8) as usize) as *mut u32;
            ptr::write_volatile(entry.add(0), 0x00000001);
            ptr::write_volatile(entry.add(1), DMA_CTX_OFFSET << 9);
        }

        self.nv_wr32(NV_PDISP_INST_TARGET, 0x00000009);
        self.nv_wr32(NV_PDISP_INST_ADDR, DISP_INST_VRAM >> 16);

        // Interrupts
        self.nv_wr32(0x611CF0, 0x00000187);
        self.nv_wr32(0x611DB0, 0x00000187);
        self.nv_wr32(0x611CEC, 0x00030001);
        self.nv_wr32(0x611DAC, 0x00000000);
        self.nv_wr32(0x611CE8, 0x000000FF);
        self.nv_wr32(0x611DA8, 0x00000000);
        self.nv_wr32(0x611CE4, 0x000000FF);
        self.nv_wr32(0x611DA4, 0x00000000);
        for i in 0..2u32 {
            self.nv_wr32(0x611CC0 + i * 4, 0x00000004);
            self.nv_wr32(0x611D80 + i * 4, 0x00000000);
        }
        self.nv_wr32(0x611CF4, 0x00000000);
        self.nv_wr32(0x611DB4, 0x00000000);
    }

    fn mode_set(&mut self) {
        // 1280x1024@60Hz VESA timing
        self.core_push(core_head(0, HEAD_SET_RASTER_SIZE), (1066 << 16) | 1688);
        self.core_push(core_head(0, HEAD_SET_RASTER_SYNC_END), (1028 << 16) | 1440);
        self.core_push(core_head(0, HEAD_SET_RASTER_BLANK_END), (39 << 16) | 48);
        self.core_push(core_head(0, HEAD_SET_RASTER_BLANK_START), (1024 << 16) | 1280);
        self.core_push(core_head(0, HEAD_SET_CONTROL), 0x00000001);
        self.core_push(CORE_UPDATE, 0x00000001);
        self.core_kick();
        self.core_wait();
    }

    fn window_bounds(&mut self) {
        self.core_push(core_win(0, WIN_SET_CONTROL_CORE), 0x00000000);
        self.core_push(core_win(0, WIN_SET_FORMAT_BOUNDS), 0x0000000F);
        self.core_push(core_win(0, WIN_SET_ROTATED_BOUNDS), 0x00000000);
        self.core_push(core_win(0, WIN_SET_USAGE_BOUNDS), 0x00007FFF | (2 << 20));
        self.core_push(CORE_UPDATE, 0x00000001);
        self.core_kick();
        self.core_wait();
    }

    fn surface_set(&mut self, vram_offset: u32) -> bool {
        let w = self.gop_width;
        let h = self.gop_height;
        let pitch = self.gop_pitch;

        let geom = [
            (h << 16) | w,    // SET_SIZE
            1 << 4,           // SET_STORAGE: PITCH
            0x000000CF,       // SET_PARAMS: A8R8G8B8
            pitch >> 6,       // SET_PLANAR_STORAGE
        ];
        let dma = [0x00000001u32, 0x00000001];

        self.win_push(WIN_SET_PRESENT_CONTROL, 0x00000001);
        self.win_push_multi(WIN_SET_SIZE, &geom);
        self.win_push_multi(WIN_SET_CTX_DMA_ISO, &dma);
        self.win_push(WIN_SET_OFFSET, vram_offset >> 8);
        self.win_push(WIN_SET_POINT_IN, 0x00000000);
        self.win_push(WIN_SET_SIZE_IN, (h << 16) | w);
        self.win_push(WIN_SET_SIZE_OUT, (h << 16) | w);
        self.win_push(WIN_UPDATE, 0x00000001);
        self.win_kick();
        self.win_wait();

        ((self.nv_rd32(nv_pdisp_win_stat(0)) >> 16) & 0xF) == 4
    }

    fn chip_name(boot0: u32) -> &'static str {
        match (boot0 >> 20) & 0x1FF {
            0x167 => "TU117",
            0x166 => "TU116",
            0x164 => "TU104",
            0x162 => "TU102",
            _ => "unknown",
        }
    }

    pub fn detect(con: &mut Console) -> Option<Self> {
        for &dev_id in KNOWN_DEVICES {
            if let Some(dev) = pci_find_vendor(NV_PCI_VENDOR, dev_id) {
                pci_enable_device(&dev);

                let bar0 = pci_read_bar(&dev, 0);
                let mmio = (bar0 & 0xFFFFFFF0) as *mut u32;

                let bar1 = pci_read_bar(&dev, 1);
                let fb_bar = (bar1 & 0xFFFFFFF0) as *mut u8;

                let boot0 = unsafe { ptr::read_volatile(mmio) };

                con.print(" NVIDIA: ");
                con.print(Self::chip_name(boot0));
                con.putchar(b'\n');

                return Some(Self {
                    mmio,
                    fb_bar,
                    gop_fb: core::ptr::null_mut(),
                    gop_width: 0,
                    gop_height: 0,
                    gop_pitch: 0,
                    core_pb: core::ptr::null_mut(),
                    core_pb_put: 0,
                    win_pb: core::ptr::null_mut(),
                    win_pb_put: 0,
                    flip_ready: false,
                });
            }
        }
        None
    }

    pub fn init(&mut self, gop_fb: *mut u8, width: u32, height: u32, pitch: u32, con: &mut Console) {
        self.gop_fb = gop_fb;
        self.gop_width = width;
        self.gop_height = height;
        self.gop_pitch = pitch;

        self.disp_engine_init();

        // Core channel
        self.core_pb = unsafe { self.fb_bar.add(CORE_PB_VRAM as usize) as *mut u32 };
        self.core_pb_put = 0;
        unsafe { ptr::write_bytes(self.core_pb, 0, PB_SIZE / 4); }

        if !self.channel_init(0, CORE_PB_VRAM, NV_PDISP_CORE_PUT, NV_PDISP_CORE_STAT) {
            con.print(" NVIDIA: core channel failed\n");
            return;
        }

        self.mode_set();
        self.window_bounds();

        // Window channel
        self.win_pb = unsafe { self.fb_bar.add(WIN_PB_VRAM as usize) as *mut u32 };
        self.win_pb_put = 0;
        unsafe { ptr::write_bytes(self.win_pb, 0, PB_SIZE / 4); }

        if !self.channel_init(1, WIN_PB_VRAM, nv_pdisp_win_put(0), 0x610664) {
            con.print(" NVIDIA: window channel failed\n");
            return;
        }

        // Clear page 1 only — page 0 has the boot text (via GOP)
        let page_size = (pitch * height) as usize;
        unsafe {
            ptr::write_bytes(gop_fb.add(PAGE1_VRAM as usize), 0, page_size);
        }

        if self.surface_set(PAGE0_VRAM) {
            self.flip_ready = true;
            con.print(" NVIDIA: surface ready\n");
        } else {
            con.print(" NVIDIA: surface config failed\n");
        }
    }

    // Page flipping disabled — dirty-region memcpy through gop_fb (WC) is faster
    // than full-frame memcpy through BAR1 (UC) + EVO flip.
    pub fn name(&self) -> &'static str { "NVIDIA" }
    pub fn framebuffer(&self) -> *mut u8 { self.gop_fb }
    pub fn width(&self) -> u32 { self.gop_width }
    pub fn height(&self) -> u32 { self.gop_height }
    pub fn pitch(&self) -> u32 { self.gop_pitch }
    pub fn can_flip(&self) -> bool { false }
    pub fn page_addr(&self, _page: u8) -> *mut u8 { self.gop_fb }
    pub fn set_page(&mut self, _page: u8) {}
    pub fn update(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {}
}
