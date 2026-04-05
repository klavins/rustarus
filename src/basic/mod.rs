// mod.rs - BASIC interpreter REPL and public API
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

mod token;
mod parser;
mod exec;
pub mod value;

use crate::console::Console;
use exec::BasicState;
use token::{TokenLine, tokenize};

/// Read a line of input from the keyboard, echoing to console.
pub fn read_line(con: &mut Console, buf: &mut [u8]) -> usize {
    let mut pos = 0;
    loop {
        let c = crate::interrupts::keybuf_read_blocking() as u8;
        if c == b'\n' {
            con.putchar(b'\n');
            return pos;
        } else if c == b'\x08' {
            if pos > 0 {
                pos -= 1;
                con.putchar(b'\x08');
            }
        } else if c != 0 && pos < buf.len() - 1 {
            buf[pos] = c;
            pos += 1;
            con.putchar(c);
        }
    }
}

fn print_error(con: &mut Console, msg: &str) {
    con.print("? ");
    con.print(msg);
    con.putchar(b'\n');
}

/// Main BASIC REPL — called from main after boot.
pub fn basic_repl(con: &mut Console) -> ! {
    static mut STATE: BasicState = BasicState::new();
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };

    parser::rng_seed();

    // Check for AUTORUN.BAS on disk
    {
        let buf = exec::BasicState::disk_buf();
        if let Ok(size) = crate::fs::fs_load(b"AUTORUN.BAS", buf, 16384) {
            state.vars = [0.0; 26];
            state.deserialize_program(buf, size);
            con.print(" AUTORUN.BAS\n");
            state.run(con);
        }
    }

    let mut line_buf = [0u8; 256];

    loop {
        con.print(" > ");
        let len = read_line(con, &mut line_buf);
        if len == 0 {
            continue;
        }

        if line_buf[0].is_ascii_digit() {
            handle_program_line(state, &line_buf, len);
        } else {
            let mut tl = TokenLine::new();
            match tokenize(&line_buf, len, &mut tl) {
                Ok(()) if tl.count > 0 => {
                    if let Err(e) = state.exec_statement(&tl, 0, con) {
                        print_error(con, e);
                    }
                }
                Err(e) => print_error(con, e),
                _ => {}
            }
        }
    }
}

fn handle_program_line(state: &mut BasicState, buf: &[u8], len: usize) {
    let mut i = 0;
    let mut linenum: u16 = 0;
    while i < len && buf[i].is_ascii_digit() {
        linenum = linenum * 10 + (buf[i] - b'0') as u16;
        i += 1;
    }
    while i < len && (buf[i] == b' ' || buf[i] == b'\t') {
        i += 1;
    }

    if i >= len {
        state.insert_line(linenum, &[]);
    } else {
        state.insert_line(linenum, &buf[i..len]);
    }
}
