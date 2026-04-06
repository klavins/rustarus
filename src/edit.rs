// edit.rs - kilo-rs editor integration for rustarus
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

extern crate alloc;
use alloc::vec::Vec;
use kilo_rs::{Editor, EditorIo, Key};
use crate::console::Console;
use crate::interrupts;
use crate::vt100::Vt100;

const MAX_FILE_SIZE: usize = 32768;

/// EditorIo implementation for rustarus.
pub struct RustarusIo<'a> {
    con: &'a mut Console,
    vt: &'a mut Vt100,
}

impl<'a> RustarusIo<'a> {
    pub fn new(con: &'a mut Console, vt: &'a mut Vt100) -> Self {
        Self { con, vt }
    }
}

impl<'a> EditorIo for RustarusIo<'a> {
    fn read_key(&mut self) -> Key {
        let c = interrupts::keybuf_read_blocking();
        match c {
            10 | 13 => Key::Enter,
            127 => Key::Backspace,
            8 => Key::CtrlH,
            17 => Key::CtrlQ,
            19 => Key::CtrlS,
            6 => Key::CtrlF,
            12 => Key::CtrlL,
            27 => Key::Escape,
            c if c == interrupts::KEY_ARROW_UP => Key::ArrowUp,
            c if c == interrupts::KEY_ARROW_DOWN => Key::ArrowDown,
            c if c == interrupts::KEY_ARROW_LEFT => Key::ArrowLeft,
            c if c == interrupts::KEY_ARROW_RIGHT => Key::ArrowRight,
            c if c == interrupts::KEY_HOME => Key::Home,
            c if c == interrupts::KEY_END => Key::End,
            c if c == interrupts::KEY_PAGE_UP => Key::PageUp,
            c if c == interrupts::KEY_PAGE_DOWN => Key::PageDown,
            c if c == interrupts::KEY_DELETE => Key::Delete,
            c if c >= 32 && c < 127 => Key::Char(c as u8),
            _ => Key::None,
        }
    }

    fn write_str(&mut self, s: &str) {
        // Route through VT100 for escape sequence interpretation
        self.vt.write(self.con, s.as_bytes());
    }

    fn load_file(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut buf = [0u8; MAX_FILE_SIZE];
        match crate::fs::fs_load(name.as_bytes(), &mut buf, MAX_FILE_SIZE) {
            Ok(size) => Some(buf[..size].to_vec()),
            Err(_) => None,
        }
    }

    fn save_file(&mut self, name: &str, data: &[u8]) -> bool {
        crate::fs::fs_save(name.as_bytes(), data).is_ok()
    }
}

/// Launch the editor. Called from the BASIC EDIT command.
pub fn run_editor(con: &mut Console, filename: Option<&str>) {
    interrupts::keybuf_flush();

    let vt = unsafe { crate::VT100.get() };
    let (rows, cols) = con.get_size();

    let mut io = RustarusIo::new(con, vt);
    let mut editor = Editor::new(rows as usize, cols as usize);

    if let Some(name) = filename {
        editor.open(name, &mut io);
    }

    editor.run(&mut io);

    // Restore console state after editor exits
    con.set_color(crate::console::Color::White, crate::console::Color::Black);
    con.clear();
}
