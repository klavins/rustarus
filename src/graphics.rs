// graphics.rs - Graphics modes, drawing primitives, and VGA palette
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
use crate::font::FONT_8X16;
use core::ptr;

static MODE_TARGET_WIDTH: [u32; 6] = [0, 0, 320, 640, 800, 0];

pub static GRAPHICS: StaticCell<Graphics> = StaticCell::new(Graphics::new());

pub struct Graphics {
    fb_addr: *mut u8,
    shadow: *mut u8,
    saved_fb: *mut u8, // save area for text screen
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_size: u32,
    mode: u8,
    virt_width: u32,
    virt_height: u32,
    pixel_scale: u32,
    offset_x: u32,
    offset_y: u32,
    cursor_x: i32,
    cursor_y: i32,
    draw_color: u32,
    draw_page: u8,
    dirty_y0: u32,
    dirty_y1: u32,
}

unsafe impl Send for Graphics {}

impl Graphics {
    pub const fn new() -> Self {
        Self {
            fb_addr: core::ptr::null_mut(),
            shadow: core::ptr::null_mut(),
            saved_fb: core::ptr::null_mut(),
            fb_width: 0,
            fb_height: 0,
            fb_pitch: 0,
            fb_size: 0,
            mode: 0,
            virt_width: 0,
            virt_height: 0,
            pixel_scale: 1,
            offset_x: 0,
            offset_y: 0,
            cursor_x: 0,
            cursor_y: 0,
            draw_color: 0x00FFFFFF,
            draw_page: 0,
            dirty_y0: 0,
            dirty_y1: 0,
        }
    }

    pub fn init(
        &mut self,
        fb_addr: *mut u8,
        shadow: *mut u8,
        saved_fb: *mut u8,
        width: u32,
        height: u32,
        pitch: u32,
    ) {
        self.fb_addr = fb_addr;
        self.shadow = shadow;
        self.saved_fb = saved_fb;
        self.fb_width = width;
        self.fb_height = height;
        self.fb_pitch = pitch;
        self.fb_size = pitch * height;
        self.mode = 0;
    }

    pub fn set_fb_addr(&mut self, fb_addr: *mut u8) {
        self.fb_addr = fb_addr;
    }

    pub fn mode(&self) -> u8 {
        self.mode
    }

    pub fn virt_width(&self) -> u32 {
        self.virt_width
    }

    pub fn virt_height(&self) -> u32 {
        self.virt_height
    }

    fn setup_virtual_res(&mut self, mode: u8) {
        if mode == 5 || MODE_TARGET_WIDTH[mode as usize] == 0 {
            self.pixel_scale = 1;
            self.virt_width = self.fb_width;
            self.virt_height = self.fb_height;
        } else {
            self.pixel_scale = self.fb_width / MODE_TARGET_WIDTH[mode as usize];
            if self.pixel_scale < 1 {
                self.pixel_scale = 1;
            }
            self.virt_width = self.fb_width / self.pixel_scale;
            self.virt_height = self.fb_height / self.pixel_scale;
        }
        self.offset_x = (self.fb_width - self.virt_width * self.pixel_scale) / 2;
        self.offset_y = (self.fb_height - self.virt_height * self.pixel_scale) / 2;
    }

    /// Switch graphics mode. Modes 0-1 are text, 2-5 are graphics.
    /// `con` is needed to save/restore the text screen.
    pub fn set_mode(&mut self, mode: u8, con: &mut crate::console::Console) {
        if mode > 5 {
            return;
        }

        let was_graphics = self.mode >= 2;
        let will_be_graphics = mode >= 2;

        if !was_graphics && will_be_graphics {
            // Text → Graphics: save text screen
            if !self.saved_fb.is_null() && !self.shadow.is_null() {
                unsafe {
                    ptr::copy_nonoverlapping(self.shadow, self.saved_fb, self.fb_size as usize);
                }
            }
            self.setup_virtual_res(mode);
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.draw_color = vga_to_rgb32(15); // white
            self.clear_buffer();
            self.present();
        } else if was_graphics && !will_be_graphics {
            // Graphics → Text: restore text screen
            if !self.saved_fb.is_null() && !self.shadow.is_null() {
                unsafe {
                    ptr::copy_nonoverlapping(self.saved_fb, self.shadow, self.fb_size as usize);
                    ptr::copy_nonoverlapping(self.shadow, self.fb_addr, self.fb_size as usize);
                }
            }
            con.set_color(crate::console::Color::White, crate::console::Color::Black);
        } else if was_graphics && will_be_graphics {
            // Graphics → Graphics: just reconfigure
            self.setup_virtual_res(mode);
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.clear_buffer();
            self.present();
        }

        self.mode = mode;
    }

    fn clear_buffer(&mut self) {
        if self.shadow.is_null() {
            return;
        }
        // Clear row by row to respect pitch (stride may exceed width * 4)
        for y in 0..self.fb_height {
            unsafe {
                let row_ptr = self.shadow.add((y * self.fb_pitch) as usize) as *mut u32;
                core::slice::from_raw_parts_mut(row_ptr, self.fb_width as usize).fill(0);
            }
        }
        self.dirty_y0 = 0;
        self.dirty_y1 = self.fb_height;
    }

    fn dirty_mark(&mut self, y0: u32, y1: u32) {
        // Empty when min == max (both 0 after reset)
        if self.dirty_y0 == self.dirty_y1 {
            self.dirty_y0 = y0;
            self.dirty_y1 = y1;
        } else {
            if y0 < self.dirty_y0 { self.dirty_y0 = y0; }
            if y1 > self.dirty_y1 { self.dirty_y1 = y1; }
        }
    }

    fn dirty_reset(&mut self) {
        self.dirty_y0 = 0;
        self.dirty_y1 = 0;
    }

    /// Write a scaled virtual pixel to the shadow buffer.
    /// Bounds-checks once, writes the entire pixel_scale x pixel_scale block,
    /// and marks dirty once.
    pub fn pixel(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 || x as u32 >= self.virt_width || y as u32 >= self.virt_height {
            return;
        }
        let px0 = self.offset_x + x as u32 * self.pixel_scale;
        let py0 = self.offset_y + y as u32 * self.pixel_scale;
        let scale = self.pixel_scale;
        unsafe {
            for sy in 0..scale {
                let row_offset = ((py0 + sy) * self.fb_pitch + px0 * 4) as usize;
                let row_ptr = self.shadow.add(row_offset) as *mut u32;
                for sx in 0..scale {
                    ptr::write(row_ptr.add(sx as usize), color);
                }
            }
        }
        self.dirty_mark(py0, py0 + scale);
    }

    pub fn plot(&mut self, x: i32, y: i32) {
        self.pixel(x, y, self.draw_color);
        self.cursor_x = x;
        self.cursor_y = y;
    }

    pub fn drawto(&mut self, x1: i32, y1: i32) {
        let x0 = self.cursor_x;
        let y0 = self.cursor_y;
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;
        let mut cx = x0;
        let mut cy = y0;
        let color = self.draw_color;

        loop {
            self.pixel(cx, cy, color);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                cx += sx;
            }
            if e2 < dx {
                err += dx;
                cy += sy;
            }
        }
        self.cursor_x = x1;
        self.cursor_y = y1;
    }

    pub fn fillto(&mut self, x1: i32, y1: i32) {
        let x0 = self.cursor_x;
        let y0 = self.cursor_y;
        let left = x0.min(x1).max(0) as u32;
        let right = (x0.max(x1) as u32).min(self.virt_width.saturating_sub(1));
        let top = y0.min(y1).max(0) as u32;
        let bottom = (y0.max(y1) as u32).min(self.virt_height.saturating_sub(1));
        let color = self.draw_color;
        let scale = self.pixel_scale;

        // Write scaled rows directly to shadow buffer
        let phys_left = self.offset_x + left * scale;
        let phys_width = (right - left + 1) * scale;
        let phys_top = self.offset_y + top * scale;
        let phys_bottom = self.offset_y + (bottom + 1) * scale;

        unsafe {
            for py in phys_top..phys_bottom {
                let row_offset = (py * self.fb_pitch + phys_left * 4) as usize;
                let row_ptr = self.shadow.add(row_offset) as *mut u32;
                core::slice::from_raw_parts_mut(row_ptr, phys_width as usize).fill(color);
            }
        }
        self.dirty_mark(phys_top, phys_bottom);
        self.cursor_x = x1;
        self.cursor_y = y1;
    }

    pub fn set_color(&mut self, idx: u8) {
        self.draw_color = vga_to_rgb32(idx);
    }

    pub fn pos(&mut self, x: i32, y: i32) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    pub fn text(&mut self, s: &[u8]) {
        let color = self.draw_color;
        for &ch in s {
            let glyph_idx = if ch < 128 { ch as usize } else { 0 };
            let glyph = &FONT_8X16[glyph_idx];
            for row in 0..16i32 {
                let bits = glyph[row as usize];
                for col in 0..8i32 {
                    if bits & (0x80 >> col) != 0 {
                        self.pixel(self.cursor_x + col, self.cursor_y + row, color);
                    }
                }
            }
            self.cursor_x += 8;
        }
    }

    pub fn present(&mut self) {
        if self.mode < 2 || self.shadow.is_null() {
            return;
        }

        let gpu = unsafe { crate::gpu::GPU.get() };

        if gpu.can_flip() {
            // Page flip path: copy shadow to back page, then flip
            let back = 1 - self.draw_page;
            let dest = gpu.page_addr(back);
            if !dest.is_null() {
                let bytes = (self.fb_pitch * self.fb_height) as usize;
                unsafe {
                    ptr::copy_nonoverlapping(self.shadow, dest, bytes);
                }
                gpu.set_page(back);
                self.draw_page = back;
            }
        } else {
            // Dirty-region path: memcpy changed rows to MMIO framebuffer
            if self.fb_addr.is_null() || self.dirty_y0 >= self.dirty_y1 {
                return;
            }
            let y0 = self.dirty_y0.min(self.fb_height);
            let y1 = self.dirty_y1.min(self.fb_height);
            let offset = (y0 * self.fb_pitch) as usize;
            let bytes = ((y1 - y0) * self.fb_pitch) as usize;
            unsafe {
                ptr::copy_nonoverlapping(
                    self.shadow.add(offset),
                    self.fb_addr.add(offset),
                    bytes,
                );
            }
            crate::gpu::gpu_update(0, y0, self.fb_width, y1 - y0);
        }
        self.dirty_reset();
    }

    pub fn clear(&mut self) {
        self.clear_buffer();
        self.cursor_x = 0;
        self.cursor_y = 0;
    }
}

/// Convert VGA DAC palette index to 32-bit BGRA.
fn vga_to_rgb32(idx: u8) -> u32 {
    let [r6, g6, b6] = VGA_DAC[idx as usize];
    let r = (r6 as u32 * 255) / 63;
    let g = (g6 as u32 * 255) / 63;
    let b = (b6 as u32 * 255) / 63;
    (r << 16) | (g << 8) | b
}

#[rustfmt::skip]
static VGA_DAC: [[u8; 3]; 256] = [
    [ 0, 0, 0],[  0, 0,42],[ 0,42, 0],[ 0,42,42],
    [42, 0, 0],[42, 0,42],[42,21, 0],[42,42,42],
    [21,21,21],[21,21,63],[21,63,21],[21,63,63],
    [63,21,21],[63,21,63],[63,63,21],[63,63,63],
    [ 0, 0, 0],[ 5, 5, 5],[ 8, 8, 8],[11,11,11],
    [14,14,14],[17,17,17],[20,20,20],[24,24,24],
    [28,28,28],[32,32,32],[36,36,36],[40,40,40],
    [45,45,45],[50,50,50],[56,56,56],[63,63,63],
    [ 0, 0,63],[16, 0,63],[31, 0,63],[47, 0,63],
    [63, 0,63],[63, 0,47],[63, 0,31],[63, 0,16],
    [63, 0, 0],[63,16, 0],[63,31, 0],[63,47, 0],
    [63,63, 0],[47,63, 0],[31,63, 0],[16,63, 0],
    [ 0,63, 0],[ 0,63,16],[ 0,63,31],[ 0,63,47],
    [ 0,63,63],[ 0,47,63],[ 0,31,63],[ 0,16,63],
    [31,31,63],[39,31,63],[47,31,63],[55,31,63],
    [63,31,63],[63,31,55],[63,31,47],[63,31,39],
    [63,31,31],[63,39,31],[63,47,31],[63,55,31],
    [63,63,31],[55,63,31],[47,63,31],[39,63,31],
    [31,63,31],[31,63,39],[31,63,47],[31,63,55],
    [31,63,63],[31,55,63],[31,47,63],[31,39,63],
    [45,45,63],[49,45,63],[54,45,63],[58,45,63],
    [63,45,63],[63,45,58],[63,45,54],[63,45,49],
    [63,45,45],[63,49,45],[63,54,45],[63,58,45],
    [63,63,45],[58,63,45],[54,63,45],[49,63,45],
    [45,63,45],[45,63,49],[45,63,54],[45,63,58],
    [45,63,63],[45,58,63],[45,54,63],[45,49,63],
    [ 0, 0,28],[ 7, 0,28],[14, 0,28],[21, 0,28],
    [28, 0,28],[28, 0,21],[28, 0,14],[28, 0, 7],
    [28, 0, 0],[28, 7, 0],[28,14, 0],[28,21, 0],
    [28,28, 0],[21,28, 0],[14,28, 0],[ 7,28, 0],
    [ 0,28, 0],[ 0,28, 7],[ 0,28,14],[ 0,28,21],
    [ 0,28,28],[ 0,21,28],[ 0,14,28],[ 0, 7,28],
    [14,14,28],[17,14,28],[21,14,28],[24,14,28],
    [28,14,28],[28,14,24],[28,14,21],[28,14,17],
    [28,14,14],[28,17,14],[28,21,14],[28,24,14],
    [28,28,14],[24,28,14],[21,28,14],[17,28,14],
    [14,28,14],[14,28,17],[14,28,21],[14,28,24],
    [14,28,28],[14,24,28],[14,21,28],[14,17,28],
    [20,20,28],[22,20,28],[24,20,28],[26,20,28],
    [28,20,28],[28,20,26],[28,20,24],[28,20,22],
    [28,20,20],[28,22,20],[28,24,20],[28,26,20],
    [28,28,20],[26,28,20],[24,28,20],[22,28,20],
    [20,28,20],[20,28,22],[20,28,24],[20,28,26],
    [20,28,28],[20,26,28],[20,24,28],[20,22,28],
    [ 0, 0,16],[ 4, 0,16],[ 8, 0,16],[12, 0,16],
    [16, 0,16],[16, 0,12],[16, 0, 8],[16, 0, 4],
    [16, 0, 0],[16, 4, 0],[16, 8, 0],[16,12, 0],
    [16,16, 0],[12,16, 0],[ 8,16, 0],[ 4,16, 0],
    [ 0,16, 0],[ 0,16, 4],[ 0,16, 8],[ 0,16,12],
    [ 0,16,16],[ 0,12,16],[ 0, 8,16],[ 0, 4,16],
    [ 8, 8,16],[10, 8,16],[12, 8,16],[14, 8,16],
    [16, 8,16],[16, 8,14],[16, 8,12],[16, 8,10],
    [16, 8, 8],[16,10, 8],[16,12, 8],[16,14, 8],
    [16,16, 8],[14,16, 8],[12,16, 8],[10,16, 8],
    [ 8,16, 8],[ 8,16,10],[ 8,16,12],[ 8,16,14],
    [ 8,16,16],[ 8,14,16],[ 8,12,16],[ 8,10,16],
    [11,11,16],[12,11,16],[13,11,16],[15,11,16],
    [16,11,16],[16,11,15],[16,11,13],[16,11,12],
    [16,11,11],[16,12,11],[16,13,11],[16,15,11],
    [16,16,11],[15,16,11],[13,16,11],[12,16,11],
    [11,16,11],[11,16,12],[11,16,13],[11,16,15],
    [11,16,16],[11,15,16],[11,13,16],[11,12,16],
    [ 0, 0, 0],[ 0, 0, 0],[ 0, 0, 0],[ 0, 0, 0],
    [ 0, 0, 0],[ 0, 0, 0],[ 0, 0, 0],[ 0, 0, 0],
];
