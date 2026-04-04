// console.rs - Framebuffer-based text console with bitmap font rendering
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

use crate::font::FONT_8X16;
use core::ptr;

const FONT_W: usize = 8;
const FONT_H: usize = 16;

/// 32-bit BGRA colors matching VGA 16-color palette
const COLOR32_MAP: [u32; 16] = [
    0x00000000, // black
    0x000000AA, // blue
    0x0000AA00, // green
    0x0000AAAA, // cyan
    0x00AA0000, // red
    0x00AA00AA, // magenta
    0x00AA5500, // brown
    0x00AAAAAA, // light gray
    0x00555555, // dark gray
    0x005555FF, // light blue
    0x0055FF55, // light green
    0x0055FFFF, // light cyan
    0x00FF5555, // light red
    0x00FF55FF, // light magenta
    0x00FFFF55, // yellow
    0x00FFFFFF, // white
];

// Safety: Console is only used from a single CPU core (no SMP),
// and the Mutex ensures exclusive access.
unsafe impl Send for Console {}

pub struct Console {
    fb_addr: *mut u8,     // real (MMIO) framebuffer
    fb_shadow: *mut u8,   // RAM shadow buffer
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,        // bytes per scan line
    fb_cols: u32,
    fb_rows: u32,
    cursor_row: u32,
    cursor_col: u32,
    wrap_pending: bool,
    fg_color: u32,
    bg_color: u32,
}

impl Console {
    /// Create an uninitialized console. Call `init` before use.
    pub const fn new() -> Self {
        Self {
            fb_addr: core::ptr::null_mut(),
            fb_shadow: core::ptr::null_mut(),
            fb_width: 0,
            fb_height: 0,
            fb_pitch: 0,
            fb_cols: 0,
            fb_rows: 0,
            cursor_row: 0,
            cursor_col: 0,
            wrap_pending: false,
            fg_color: COLOR32_MAP[15], // white
            bg_color: COLOR32_MAP[0],  // black
        }
    }

    /// Initialize from a UEFI GOP framebuffer.
    /// `shadow` must point to an allocation of at least `pitch * height` bytes.
    pub fn init(
        &mut self,
        fb_addr: *mut u8,
        shadow: *mut u8,
        width: u32,
        height: u32,
        pitch: u32,
    ) {
        self.fb_addr = fb_addr;
        self.fb_shadow = shadow;
        self.fb_width = width;
        self.fb_height = height;
        self.fb_pitch = pitch;
        self.fb_cols = width / FONT_W as u32;
        self.fb_rows = height / FONT_H as u32;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.wrap_pending = false;
        self.fg_color = COLOR32_MAP[15];
        self.bg_color = COLOR32_MAP[0];
        self.clear();
    }

    fn buf(&self) -> *mut u8 {
        if !self.fb_shadow.is_null() {
            self.fb_shadow
        } else {
            self.fb_addr
        }
    }

    fn pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.fb_width || y >= self.fb_height {
            return;
        }
        let offset = (y * self.fb_pitch + x * 4) as usize;
        unsafe {
            let p = self.buf().add(offset) as *mut u32;
            ptr::write(p, color);
        }
    }

    fn flush_region(&self, y0: u32, rows: u32) {
        if self.fb_shadow.is_null() {
            return;
        }
        let offset = (y0 * self.fb_pitch) as usize;
        let bytes = (rows * self.fb_pitch) as usize;
        unsafe {
            ptr::copy_nonoverlapping(
                self.fb_shadow.add(offset),
                self.fb_addr.add(offset),
                bytes,
            );
        }
    }

    fn flush_all(&self) {
        if self.fb_shadow.is_null() {
            return;
        }
        let bytes = (self.fb_pitch * self.fb_height) as usize;
        unsafe {
            ptr::copy_nonoverlapping(self.fb_shadow, self.fb_addr, bytes);
        }
    }

    fn draw_char(&self, col: u32, row: u32, ch: u8, fg: u32, bg: u32) {
        let x0 = col * FONT_W as u32;
        let y0 = row * FONT_H as u32;
        let glyph = if ch < 128 {
            &FONT_8X16[ch as usize]
        } else {
            &FONT_8X16[0]
        };
        for y in 0..FONT_H as u32 {
            let bits = glyph[y as usize];
            for x in 0..FONT_W as u32 {
                let color = if bits & (0x80 >> x) != 0 { fg } else { bg };
                self.pixel(x0 + x, y0 + y, color);
            }
        }
    }

    fn draw_cursor(&self, show: bool) {
        let x0 = self.cursor_col * FONT_W as u32;
        let y0 = self.cursor_row * FONT_H as u32 + FONT_H as u32 - 2;
        let color = if show { self.fg_color } else { self.bg_color };
        for y in 0..2u32 {
            for x in 0..FONT_W as u32 {
                self.pixel(x0 + x, y0 + y, color);
            }
        }
    }

    fn scroll(&self) {
        let buf = self.buf();
        let row_bytes = self.fb_pitch * FONT_H as u32;
        let total = self.fb_pitch * (self.fb_height - FONT_H as u32);
        unsafe {
            ptr::copy(buf.add(row_bytes as usize), buf, total as usize);
            // Clear last row
            let last = buf.add((self.fb_pitch * (self.fb_height - FONT_H as u32)) as usize);
            let pixels = self.fb_width * FONT_H as u32;
            let p = last as *mut u32;
            for i in 0..pixels {
                ptr::write(p.add(i as usize), self.bg_color);
            }
        }
        self.flush_all();
    }

    pub fn clear(&mut self) {
        let buf = self.buf();
        let total = self.fb_width * self.fb_height;
        unsafe {
            let p = buf as *mut u32;
            for i in 0..total {
                ptr::write(p.add(i as usize), self.bg_color);
            }
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.wrap_pending = false;
        self.draw_cursor(true);
        self.flush_all();
    }

    pub fn putchar(&mut self, c: u8) {
        self.draw_cursor(false);

        if c == b'\r' {
            self.cursor_col = 0;
            self.wrap_pending = false;
            self.draw_cursor(true);
            return;
        }
        if c == b'\x08' {
            // backspace
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
                self.wrap_pending = false;
                self.draw_char(self.cursor_col, self.cursor_row, b' ', self.fg_color, self.bg_color);
            }
            self.draw_cursor(true);
            self.flush_region(self.cursor_row * FONT_H as u32, FONT_H as u32);
            return;
        }
        if c == b'\n' {
            let old_row = self.cursor_row;
            self.cursor_col = 0;
            self.wrap_pending = false;
            self.cursor_row += 1;
            if self.cursor_row >= self.fb_rows {
                self.scroll();
                self.cursor_row = self.fb_rows - 1;
            } else {
                self.flush_region(old_row * FONT_H as u32, FONT_H as u32);
                self.flush_region(self.cursor_row * FONT_H as u32, FONT_H as u32);
            }
            self.draw_cursor(true);
            self.flush_region(self.cursor_row * FONT_H as u32, FONT_H as u32);
            return;
        }

        // Deferred wrap
        if self.wrap_pending {
            self.wrap_pending = false;
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.fb_rows {
                self.scroll();
                self.cursor_row = self.fb_rows - 1;
            }
        }

        let char_row = self.cursor_row;
        self.draw_char(self.cursor_col, self.cursor_row, c, self.fg_color, self.bg_color);
        self.cursor_col += 1;
        if self.cursor_col >= self.fb_cols {
            self.cursor_col = self.fb_cols - 1;
            self.wrap_pending = true;
        }
        self.draw_cursor(true);
        self.flush_region(char_row * FONT_H as u32, FONT_H as u32);
        if self.cursor_row != char_row {
            self.flush_region(self.cursor_row * FONT_H as u32, FONT_H as u32);
        }
    }

    pub fn print(&mut self, s: &str) {
        for b in s.bytes() {
            self.putchar(b);
        }
    }

    pub fn set_color(&mut self, fg: usize, bg: usize) {
        self.fg_color = COLOR32_MAP[fg & 0x0F];
        self.bg_color = COLOR32_MAP[bg & 0x0F];
    }
}
