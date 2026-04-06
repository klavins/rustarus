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

fn noop_gpu_update(_: u32, _: u32, _: u32, _: u32) {}

const VT_MAX_PARAMS: usize = 4;

#[derive(Copy, Clone, PartialEq)]
enum VtState {
    Normal,
    Esc,
    Csi,
    Qmark,
}

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
    // GPU update callback (set after gpu_init, avoids circular dependency)
    gpu_update_fn: fn(u32, u32, u32, u32),
    // VT100 state (inlined to avoid separate global)
    vt_state: VtState,
    vt_params: [u16; VT_MAX_PARAMS],
    vt_nparams: usize,
    vt_fg: Color,
    vt_bg: Color,
    vt_default_fg: Color,
    vt_default_bg: Color,
    vt_reverse: bool,
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
            gpu_update_fn: noop_gpu_update,
            vt_state: VtState::Normal,
            vt_params: [0; VT_MAX_PARAMS],
            vt_nparams: 0,
            vt_fg: Color::LightGray,
            vt_bg: Color::Black,
            vt_default_fg: Color::LightGray,
            vt_default_bg: Color::Black,
            vt_reverse: false,
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
        (self.gpu_update_fn)(0, y0, self.fb_width, rows);
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
        (self.gpu_update_fn)(0, 0, self.fb_width, self.fb_height);
    }

    pub fn set_gpu_update(&mut self, f: fn(u32, u32, u32, u32)) {
        self.gpu_update_fn = f;
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
        self.vt_process(c);
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

    // --- VT100 escape sequence processing (inlined from vt100.rs) ---

    /// Process a buffer of characters through the VT100 state machine.
    /// Batches output with flush_hold/release for flicker-free rendering.
    pub fn vt_write(&mut self, buf: &[u8]) {
        self.flush_hold();
        for &c in buf {
            self.vt_process(c);
        }
        self.flush_release();
    }

    fn vt_process(&mut self, c: u8) {
        match self.vt_state {
            VtState::Normal => {
                if c == 0x1B {
                    self.vt_state = VtState::Esc;
                } else {
                    self.draw_cursor(false);
                    self.putchar_inner(c);
                }
            }
            VtState::Esc => {
                if c == b'[' {
                    self.vt_state = VtState::Csi;
                    self.vt_nparams = 1;
                    self.vt_params = [0; VT_MAX_PARAMS];
                } else {
                    self.vt_state = VtState::Normal;
                }
            }
            VtState::Csi => {
                if c == b'?' {
                    self.vt_state = VtState::Qmark;
                } else {
                    self.vt_handle_csi(c);
                }
            }
            VtState::Qmark => {
                self.vt_handle_qmark(c);
            }
        }
    }

    fn vt_accumulate_digit(&mut self, digit: u8) {
        if self.vt_nparams > 0 && self.vt_nparams <= VT_MAX_PARAMS {
            let p = &mut self.vt_params[self.vt_nparams - 1];
            *p = p.saturating_mul(10).saturating_add((digit - b'0') as u16);
        }
    }

    fn vt_handle_csi(&mut self, c: u8) {
        if c.is_ascii_digit() {
            self.vt_accumulate_digit(c);
            return;
        }
        if c == b';' {
            if self.vt_nparams < VT_MAX_PARAMS {
                self.vt_nparams += 1;
            }
            return;
        }
        match c {
            b'H' | b'f' => {
                let row = if self.vt_params[0] > 0 { self.vt_params[0] - 1 } else { 0 };
                let col = if self.vt_nparams >= 2 && self.vt_params[1] > 0 { self.vt_params[1] - 1 } else { 0 };
                self.set_cursor(row as u32, col as u32);
            }
            b'A' => {
                let n = if self.vt_params[0] > 0 { self.vt_params[0] } else { 1 };
                let (row, col) = self.get_cursor();
                self.set_cursor(row.saturating_sub(n as u32), col);
            }
            b'B' => {
                let n = if self.vt_params[0] > 0 { self.vt_params[0] } else { 1 };
                let (row, col) = self.get_cursor();
                self.set_cursor(row + n as u32, col);
            }
            b'C' => {
                let n = if self.vt_params[0] > 0 { self.vt_params[0] } else { 1 };
                let (row, col) = self.get_cursor();
                self.set_cursor(row, col + n as u32);
            }
            b'D' => {
                let n = if self.vt_params[0] > 0 { self.vt_params[0] } else { 1 };
                let (row, col) = self.get_cursor();
                self.set_cursor(row, col.saturating_sub(n as u32));
            }
            b'J' => {
                if self.vt_params[0] == 2 { self.clear(); }
            }
            b'K' => {
                self.clear_to_eol();
            }
            b'm' => {
                self.vt_handle_sgr();
            }
            _ => {}
        }
        self.vt_state = VtState::Normal;
    }

    fn vt_handle_qmark(&mut self, c: u8) {
        if c.is_ascii_digit() {
            self.vt_accumulate_digit(c);
            return;
        }
        if c == b';' {
            if self.vt_nparams < VT_MAX_PARAMS { self.vt_nparams += 1; }
            return;
        }
        match c {
            b'h' => { if self.vt_params[0] == 25 { self.show_cursor(true); } }
            b'l' => { if self.vt_params[0] == 25 { self.show_cursor(false); } }
            _ => {}
        }
        self.vt_state = VtState::Normal;
    }

    fn vt_handle_sgr(&mut self) {
        for i in 0..self.vt_nparams {
            let p = self.vt_params[i];
            match p {
                0 => {
                    self.vt_fg = self.vt_default_fg;
                    self.vt_bg = self.vt_default_bg;
                    self.vt_reverse = false;
                }
                7 => { self.vt_reverse = true; }
                27 => { self.vt_reverse = false; }
                39 => self.vt_fg = self.vt_default_fg,
                49 => self.vt_bg = self.vt_default_bg,
                30..=37 => self.vt_fg = ansi_to_color((p - 30) as u8),
                40..=47 => self.vt_bg = ansi_to_color((p - 40) as u8),
                90..=97 => self.vt_fg = ansi_to_color((p - 90 + 8) as u8),
                100..=107 => self.vt_bg = ansi_to_color((p - 100 + 8) as u8),
                _ => {}
            }
        }
        if self.vt_reverse {
            self.set_color(self.vt_bg, self.vt_fg);
        } else {
            self.set_color(self.vt_fg, self.vt_bg);
        }
    }
}

fn ansi_to_color(idx: u8) -> Color {
    match idx {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Brown,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::LightGray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::Yellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        15 => Color::White,
        _ => Color::LightGray,
    }
}
