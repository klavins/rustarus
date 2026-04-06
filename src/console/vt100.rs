// vt100.rs - VT100/ANSI escape sequence interpreter
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

use crate::console::{Console, Color};

const MAX_PARAMS: usize = 4;

#[derive(Copy, Clone, PartialEq)]
enum State {
    Normal,
    Esc,     // received ESC
    Csi,     // received ESC [
    Qmark,   // received ESC [ ?
}

pub struct Vt100 {
    state: State,
    params: [u16; MAX_PARAMS],
    nparams: usize,
    fg: Color,
    bg: Color,
    default_fg: Color,
    default_bg: Color,
    reverse: bool,
}

impl Vt100 {
    pub const fn new() -> Self {
        Self {
            state: State::Normal,
            params: [0; MAX_PARAMS],
            nparams: 0,
            fg: Color::LightGray,
            bg: Color::Black,
            default_fg: Color::LightGray,
            default_bg: Color::Black,
            reverse: false,
        }
    }

    /// Process a buffer of characters through the VT100 state machine.
    /// Batches all output with flush_hold/release for flicker-free rendering.
    pub fn write(&mut self, con: &mut Console, buf: &[u8]) {
        con.flush_hold();
        for &c in buf {
            self.process(con, c);
        }
        con.flush_release();
    }

    pub fn process(&mut self, con: &mut Console, c: u8) {
        match self.state {
            State::Normal => {
                if c == 0x1B {
                    self.state = State::Esc;
                } else {
                    con.putchar_no_serial(c);
                }
            }
            State::Esc => {
                if c == b'[' {
                    self.state = State::Csi;
                    self.nparams = 1;
                    self.params = [0; MAX_PARAMS];
                } else {
                    // Unrecognized escape — discard
                    self.state = State::Normal;
                }
            }
            State::Csi => {
                if c == b'?' {
                    self.state = State::Qmark;
                } else {
                    self.handle_csi_char(con, c);
                }
            }
            State::Qmark => {
                self.handle_qmark_char(con, c);
            }
        }
    }

    fn accumulate_digit(&mut self, digit: u8) {
        if self.nparams > 0 && self.nparams <= MAX_PARAMS {
            let p = &mut self.params[self.nparams - 1];
            *p = p.saturating_mul(10).saturating_add((digit - b'0') as u16);
        }
    }

    fn handle_csi_char(&mut self, con: &mut Console, c: u8) {
        if c.is_ascii_digit() {
            self.accumulate_digit(c);
            return;
        }
        if c == b';' {
            if self.nparams < MAX_PARAMS {
                self.nparams += 1;
            }
            return;
        }

        // Dispatch on final character
        match c {
            b'H' | b'f' => {
                // Cursor Position: ESC[row;colH (1-indexed, default 1,1)
                let row = if self.params[0] > 0 { self.params[0] - 1 } else { 0 };
                let col = if self.nparams >= 2 && self.params[1] > 0 { self.params[1] - 1 } else { 0 };
                con.set_cursor(row as u32, col as u32);
            }
            b'A' => {
                // Cursor Up
                let n = if self.params[0] > 0 { self.params[0] } else { 1 };
                let (row, col) = con.get_cursor();
                con.set_cursor(row.saturating_sub(n as u32), col);
            }
            b'B' => {
                // Cursor Down
                let n = if self.params[0] > 0 { self.params[0] } else { 1 };
                let (row, col) = con.get_cursor();
                con.set_cursor(row + n as u32, col);
            }
            b'C' => {
                // Cursor Forward (right)
                let n = if self.params[0] > 0 { self.params[0] } else { 1 };
                let (row, col) = con.get_cursor();
                con.set_cursor(row, col + n as u32);
            }
            b'D' => {
                // Cursor Backward (left)
                let n = if self.params[0] > 0 { self.params[0] } else { 1 };
                let (row, col) = con.get_cursor();
                con.set_cursor(row, col.saturating_sub(n as u32));
            }
            b'J' => {
                // Erase in Display
                if self.params[0] == 2 {
                    con.clear();
                }
            }
            b'K' => {
                // Erase in Line (0 = to end of line)
                con.clear_to_eol();
            }
            b'm' => {
                // SGR — Select Graphic Rendition
                self.handle_sgr(con);
            }
            _ => {} // unrecognized — ignore
        }
        self.state = State::Normal;
    }

    fn handle_qmark_char(&mut self, con: &mut Console, c: u8) {
        if c.is_ascii_digit() {
            self.accumulate_digit(c);
            return;
        }
        if c == b';' {
            if self.nparams < MAX_PARAMS { self.nparams += 1; }
            return;
        }

        match c {
            b'h' => {
                if self.params[0] == 25 { con.show_cursor(true); }
            }
            b'l' => {
                if self.params[0] == 25 { con.show_cursor(false); }
            }
            _ => {}
        }
        self.state = State::Normal;
    }

    fn handle_sgr(&mut self, con: &mut Console) {
        // Process all parameters (SGR can have multiple: ESC[0;33;42m)
        for i in 0..self.nparams {
            let p = self.params[i];
            match p {
                0 => {
                    // Reset
                    self.fg = self.default_fg;
                    self.bg = self.default_bg;
                    self.reverse = false;
                }
                7 => {
                    // Reverse video
                    self.reverse = true;
                }
                27 => {
                    // Reverse off
                    self.reverse = false;
                }
                39 => self.fg = self.default_fg,
                49 => self.bg = self.default_bg,
                30..=37 => self.fg = ansi_to_color((p - 30) as u8),
                40..=47 => self.bg = ansi_to_color((p - 40) as u8),
                90..=97 => self.fg = ansi_to_color((p - 90 + 8) as u8),
                100..=107 => self.bg = ansi_to_color((p - 100 + 8) as u8),
                _ => {}
            }
        }
        // Apply colors (with reverse swap)
        if self.reverse {
            con.set_color(self.bg, self.fg);
        } else {
            con.set_color(self.fg, self.bg);
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
