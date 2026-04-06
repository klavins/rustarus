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

use crate::console::font::FONT_8X16;
use core::ptr;

const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

/// VGA 16-color palette.
#[derive(Copy, Clone)]
#[repr(u8)]
#[allow(dead_code)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    LightMagenta = 13,
    Yellow = 14,
    White = 15,
}

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

// Safety: Console contains raw pointers but is only used from a single CPU core.
unsafe impl Send for Console {}

pub struct Console {
    fb_addr: *mut u8,     // real (MMIO) framebuffer
    fb_shadow: *mut u8,   // RAM shadow buffer
    buf: *mut u8,         // cached active buffer (shadow if available, else fb_addr)
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,        // bytes per scan line
    fb_cols: u32,
    fb_rows: u32,
    cursor_row: u32,
    cursor_col: u32,
    wrap_pending: bool,
    flush_held: bool,
    dirty_min_y: u32,     // top of dirty region (pixel row)
    dirty_max_y: u32,     // bottom of dirty region (pixel row, exclusive)
    fg_color: u32,
    bg_color: u32,
}

impl Console {
    pub const fn new() -> Self {
        Self {
            fb_addr: core::ptr::null_mut(),
            fb_shadow: core::ptr::null_mut(),
            buf: core::ptr::null_mut(),
            fb_width: 0,
            fb_height: 0,
            fb_pitch: 0,
            fb_cols: 0,
            fb_rows: 0,
            cursor_row: 0,
            cursor_col: 0,
            wrap_pending: false,
            flush_held: false,
            dirty_min_y: 0,
            dirty_max_y: 0,
            fg_color: COLOR32_MAP[15],
            bg_color: COLOR32_MAP[0],
        }
    }

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
        self.buf = if !shadow.is_null() { shadow } else { fb_addr };
        self.fb_width = width;
        self.fb_height = height;
        self.fb_pitch = pitch;
        self.fb_cols = width / FONT_W;
        self.fb_rows = height / FONT_H;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.wrap_pending = false;
        self.flush_held = false;
        self.dirty_min_y = 0;
        self.dirty_max_y = 0;
        self.fg_color = COLOR32_MAP[15];
        self.bg_color = COLOR32_MAP[0];
        self.clear();
    }

    /// Switch the MMIO framebuffer target (used when a GPU driver takes over).
    /// The shadow buffer stays the same — only the flush destination changes.
    pub fn set_fb_addr(&mut self, fb_addr: *mut u8) {
        self.fb_addr = fb_addr;
    }

    pub fn fb_addr(&self) -> *mut u8 {
        self.fb_addr
    }

    fn pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.fb_width || y >= self.fb_height {
            return;
        }
        let offset = (y * self.fb_pitch + x * 4) as usize;
        unsafe {
            let p = self.buf.add(offset) as *mut u32;
            ptr::write(p, color);
        }
    }

    fn flush_region(&mut self, y0: u32, rows: u32) {
        if self.fb_shadow.is_null() {
            return;
        }
        if self.flush_held {
            // Expand dirty region
            let y_end = y0 + rows;
            if self.dirty_min_y == self.dirty_max_y {
                self.dirty_min_y = y0;
                self.dirty_max_y = y_end;
            } else {
                if y0 < self.dirty_min_y {
                    self.dirty_min_y = y0;
                }
                if y_end > self.dirty_max_y {
                    self.dirty_max_y = y_end;
                }
            }
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
        crate::drivers::gpu::gpu_update(0, y0, self.fb_width, rows);
    }

    fn flush_all(&mut self) {
        if self.fb_shadow.is_null() {
            return;
        }
        if self.flush_held {
            self.dirty_min_y = 0;
            self.dirty_max_y = self.fb_height;
            return;
        }
        let bytes = (self.fb_pitch * self.fb_height) as usize;
        unsafe {
            ptr::copy_nonoverlapping(self.fb_shadow, self.fb_addr, bytes);
        }
        crate::drivers::gpu::gpu_update(0, 0, self.fb_width, self.fb_height);
    }

    pub fn flush_hold(&mut self) {
        self.flush_held = true;
        self.dirty_min_y = 0;
        self.dirty_max_y = 0;
    }

    pub fn flush_release(&mut self) {
        self.flush_held = false;
        if self.dirty_min_y < self.dirty_max_y {
            let y0 = self.dirty_min_y;
            let rows = self.dirty_max_y - self.dirty_min_y;
            self.flush_region(y0, rows);
        }
    }

    fn draw_char(&self, col: u32, row: u32, ch: u8, fg: u32, bg: u32) {
        let x0 = col * FONT_W;
        let y0 = row * FONT_H;
        let glyph = if ch < 128 {
            &FONT_8X16[ch as usize]
        } else {
            &FONT_8X16[0]
        };
        for y in 0..FONT_H {
            let bits = glyph[y as usize];
            let row_offset = ((y0 + y) * self.fb_pitch + x0 * 4) as usize;
            unsafe {
                let row_ptr = self.buf.add(row_offset) as *mut u32;
                for x in 0..FONT_W {
                    let color = if bits & (0x80 >> x) != 0 { fg } else { bg };
                    ptr::write(row_ptr.add(x as usize), color);
                }
            }
        }
    }

    fn draw_cursor(&self, show: bool) {
        let x0 = self.cursor_col * FONT_W;
        let y0 = self.cursor_row * FONT_H + FONT_H - 2;
        let color = if show { self.fg_color } else { self.bg_color };
        for y in 0..2u32 {
            for x in 0..FONT_W {
                self.pixel(x0 + x, y0 + y, color);
            }
        }
    }

    fn clear_rows(&self, pixel_y: u32, rows: u32) {
        for y in pixel_y..pixel_y + rows {
            if y >= self.fb_height {
                break;
            }
            unsafe {
                let row_ptr = self.buf.add((y * self.fb_pitch) as usize) as *mut u32;
                let row = core::slice::from_raw_parts_mut(row_ptr, self.fb_width as usize);
                row.fill(self.bg_color);
            }
        }
    }

    fn scroll(&mut self) {
        let row_bytes = self.fb_pitch * FONT_H;
        let total = self.fb_pitch * (self.fb_height - FONT_H);
        unsafe {
            ptr::copy(self.buf.add(row_bytes as usize), self.buf, total as usize);
        }
        self.clear_rows(self.fb_height - FONT_H, FONT_H);
        self.flush_all();
    }

    pub fn clear(&mut self) {
        self.clear_rows(0, self.fb_height);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.wrap_pending = false;
        self.draw_cursor(true);
        self.flush_all();
    }

    pub fn putchar(&mut self, c: u8) {
        // Mirror to serial port for test capture
        if c == b'\n' { crate::console::serial::serial_putchar(b'\r'); }
        crate::console::serial::serial_putchar(c);
        // Route through VT100 interpreter
        let vt = unsafe { crate::VT100.get() };
        vt.process(self, c);
    }

    fn putchar_inner(&mut self, c: u8) {
        if c == b'\r' {
            self.cursor_col = 0;
            self.wrap_pending = false;
            self.draw_cursor(true);
            return;
        }
        if c == b'\x08' {
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
                self.wrap_pending = false;
                self.draw_char(self.cursor_col, self.cursor_row, b' ', self.fg_color, self.bg_color);
            }
            self.draw_cursor(true);
            self.flush_region(self.cursor_row * FONT_H, FONT_H);
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
                self.flush_region(old_row * FONT_H, FONT_H);
            }
            self.draw_cursor(true);
            self.flush_region(self.cursor_row * FONT_H, FONT_H);
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
        self.flush_region(char_row * FONT_H, FONT_H);
        if self.cursor_row != char_row {
            self.flush_region(self.cursor_row * FONT_H, FONT_H);
        }
    }

    pub fn print(&mut self, s: &str) {
        self.flush_hold();
        for b in s.bytes() {
            self.putchar(b);
        }
        self.flush_release();
    }

    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.fg_color = COLOR32_MAP[fg as usize];
        self.bg_color = COLOR32_MAP[bg as usize];
    }

    /// Putchar without serial mirror (for VT100 output that's already captured).
    pub fn putchar_no_serial(&mut self, c: u8) {
        self.draw_cursor(false);
        self.putchar_inner(c);
    }

    pub fn set_cursor(&mut self, row: u32, col: u32) {
        self.draw_cursor(false);
        let row = row.min(self.fb_rows.saturating_sub(1));
        let col = col.min(self.fb_cols.saturating_sub(1));
        self.cursor_row = row;
        self.cursor_col = col;
        self.wrap_pending = false;
        self.draw_cursor(true);
        self.flush_region(self.cursor_row * FONT_H, FONT_H);
    }

    pub fn get_cursor(&self) -> (u32, u32) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.fb_rows, self.fb_cols)
    }

    pub fn clear_to_eol(&mut self) {
        self.draw_cursor(false);
        for c in self.cursor_col..self.fb_cols {
            self.draw_char(c, self.cursor_row, b' ', self.fg_color, self.bg_color);
        }
        self.draw_cursor(true);
        self.flush_region(self.cursor_row * FONT_H, FONT_H);
    }

    pub fn show_cursor(&mut self, show: bool) {
        self.draw_cursor(show);
        self.flush_region(self.cursor_row * FONT_H, FONT_H);
    }

}
