// exec.rs - BASIC statement executor, program storage, and control flow
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
use super::parser::Parser;
use super::token::{TokenKind, TokenLine, tokenize, var_index};
use super::value::{print_f64, format_u64};

const PROGRAM_BUF_SIZE: usize = 16384;

const MAX_LINES: usize = 1000;
const MAX_LINE_LEN: usize = 256;

#[derive(Copy, Clone)]
struct ProgramLine {
    number: u16,
    text: [u8; MAX_LINE_LEN],
    len: usize,
}

impl ProgramLine {
    const fn empty() -> Self {
        Self { number: 0, text: [0; MAX_LINE_LEN], len: 0 }
    }
}

#[derive(Copy, Clone)]
struct ForFrame {
    var_idx: usize,
    limit: f64,
    step: f64,
    line_idx: usize,
}

impl ForFrame {
    const fn empty() -> Self {
        Self { var_idx: 0, limit: 0.0, step: 1.0, line_idx: 0 }
    }
}

const MAX_ARRAYS: usize = 32;
const MAX_ARRAY_ELEMS: usize = 1024;
const MAX_STRINGS: usize = 32;
const MAX_STR_LEN: usize = 256;
const MAX_DATA: usize = 512;

#[derive(Copy, Clone)]
pub struct Array {
    name: u8,       // A-Z index (0-25)
    dim1: u16,
    dim2: u16,      // 0 = 1D
    vals: [f64; MAX_ARRAY_ELEMS],
}

impl Array {
    const fn empty() -> Self {
        Self { name: 0, dim1: 0, dim2: 0, vals: [0.0; MAX_ARRAY_ELEMS] }
    }
}

#[derive(Copy, Clone)]
pub struct StringVar {
    name: u8,       // A-Z index (0-25)
    dimmed: bool,
    buf: [u8; MAX_STR_LEN],
    len: usize,
}

impl StringVar {
    const fn empty() -> Self {
        Self { name: 0, dimmed: false, buf: [0; MAX_STR_LEN], len: 0 }
    }
}

#[derive(Copy, Clone)]
struct DataItem {
    is_string: bool,
    num_val: f64,
    str_buf: [u8; 64],
    str_len: usize,
}

impl DataItem {
    const fn empty() -> Self {
        Self { is_string: false, num_val: 0.0, str_buf: [0; 64], str_len: 0 }
    }
}

pub struct BasicState {
    pub vars: [f64; 26],
    lines: [ProgramLine; MAX_LINES],
    line_count: usize,
    for_stack: [ForFrame; 32],
    for_sp: usize,
    gosub_stack: [usize; 64],
    gosub_sp: usize,
    running: bool,
    pc: usize,
    arrays: [Array; MAX_ARRAYS],
    array_count: usize,
    strings: [StringVar; MAX_STRINGS],
    string_count: usize,
    data_store: [DataItem; MAX_DATA],
    data_count: usize,
    data_ptr: usize,
}

impl BasicState {
    pub const fn new() -> Self {
        Self {
            vars: [0.0; 26],
            lines: [ProgramLine::empty(); MAX_LINES],
            line_count: 0,
            for_stack: [ForFrame::empty(); 32],
            for_sp: 0,
            gosub_stack: [0; 64],
            gosub_sp: 0,
            running: false,
            pc: 0,
            arrays: [Array::empty(); MAX_ARRAYS],
            array_count: 0,
            strings: [StringVar::empty(); MAX_STRINGS],
            string_count: 0,
            data_store: [DataItem::empty(); MAX_DATA],
            data_count: 0,
            data_ptr: 0,
        }
    }

    fn find_line_or_after(&self, number: u16) -> usize {
        let mut lo = 0usize;
        let mut hi = self.line_count;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.lines[mid].number < number {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn find_line(&self, number: u16) -> Option<usize> {
        let idx = self.find_line_or_after(number);
        if idx < self.line_count && self.lines[idx].number == number {
            Some(idx)
        } else {
            None
        }
    }

    fn jump_to_line(&mut self, linenum: u16) -> Result<(), &'static str> {
        match self.find_line(linenum) {
            Some(idx) => { self.pc = idx; Ok(()) }
            None => Err("UNDEF LINE"),
        }
    }

    pub fn insert_line(&mut self, number: u16, text: &[u8]) {
        if text.is_empty() {
            if let Some(idx) = self.find_line(number) {
                for i in idx..self.line_count - 1 {
                    self.lines[i] = self.lines[i + 1];
                }
                self.line_count -= 1;
            }
            return;
        }

        if let Some(idx) = self.find_line(number) {
            let copy_len = text.len().min(MAX_LINE_LEN);
            self.lines[idx].text[..copy_len].copy_from_slice(&text[..copy_len]);
            self.lines[idx].len = copy_len;
        } else if self.line_count < MAX_LINES {
            let pos = self.find_line_or_after(number);
            for i in (pos..self.line_count).rev() {
                self.lines[i + 1] = self.lines[i];
            }
            self.lines[pos] = ProgramLine::empty();
            self.lines[pos].number = number;
            let copy_len = text.len().min(MAX_LINE_LEN);
            self.lines[pos].text[..copy_len].copy_from_slice(&text[..copy_len]);
            self.lines[pos].len = copy_len;
            self.line_count += 1;
        }
    }

    pub fn list(&self, con: &mut Console) {
        for i in 0..self.line_count {
            let line = &self.lines[i];
            print_f64(con, line.number as f64);
            con.putchar(b' ');
            for j in 0..line.len {
                con.putchar(line.text[j]);
            }
            con.putchar(b'\n');
        }
    }

    pub fn clear(&mut self) {
        self.line_count = 0;
        self.vars = [0.0; 26];
        self.for_sp = 0;
        self.gosub_sp = 0;
        self.array_count = 0;
        self.string_count = 0;
        self.data_count = 0;
        self.data_ptr = 0;
    }

    // Array access
    fn array_find(&self, name_idx: usize) -> Option<usize> {
        for a in 0..self.array_count {
            if self.arrays[a].name == name_idx as u8 { return Some(a); }
        }
        None
    }

    fn array_flat_index(arr: &Array, i1: usize, i2: usize) -> Result<usize, &'static str> {
        if arr.dim2 > 0 {
            if i1 >= arr.dim1 as usize || i2 >= arr.dim2 as usize { return Err("BAD SUBSCRIPT"); }
            Ok(i1 * arr.dim2 as usize + i2)
        } else {
            if i1 >= arr.dim1 as usize { return Err("BAD SUBSCRIPT"); }
            Ok(i1)
        }
    }

    pub fn array_get(&self, name_idx: usize, i1: usize, i2: usize) -> Result<f64, &'static str> {
        let a = self.array_find(name_idx).ok_or("ARRAY NOT DIMMED")?;
        let idx = Self::array_flat_index(&self.arrays[a], i1, i2)?;
        Ok(self.arrays[a].vals[idx])
    }

    fn array_set(&mut self, name_idx: usize, i1: usize, i2: usize, val: f64) -> Result<(), &'static str> {
        let a = self.array_find(name_idx).ok_or("ARRAY NOT DIMMED")?;
        let idx = Self::array_flat_index(&self.arrays[a], i1, i2)?;
        self.arrays[a].vals[idx] = val;
        Ok(())
    }

    fn string_get(&self, name_idx: usize) -> &[u8] {
        for s in 0..self.string_count {
            if self.strings[s].name == name_idx as u8 {
                return &self.strings[s].buf[..self.strings[s].len];
            }
        }
        &[]
    }

    fn string_set(&mut self, name_idx: usize, val: &[u8]) -> Result<(), &'static str> {
        for s in 0..self.string_count {
            if self.strings[s].name == name_idx as u8 {
                let sv = &mut self.strings[s];
                if !sv.dimmed { return Err("STRING NOT DIMMED"); }
                let copy_len = val.len().min(MAX_STR_LEN - 1);
                sv.buf[..copy_len].copy_from_slice(&val[..copy_len]);
                sv.len = copy_len;
                return Ok(());
            }
        }
        Err("STRING NOT DIMMED")
    }

    fn collect_data(&mut self) {
        self.data_count = 0;
        self.data_ptr = 0;
        for i in 0..self.line_count {
            let line = &self.lines[i];
            let mut tl = TokenLine::new();
            if tokenize(&line.text, line.len, &mut tl).is_err() { continue; }
            if tl.count == 0 || tl.get(0).kind != TokenKind::Data { continue; }
            let mut pos = 1;
            while tl.get(pos).kind != TokenKind::Eol && self.data_count < MAX_DATA {
                if tl.get(pos).kind == TokenKind::Comma { pos += 1; continue; }
                if tl.get(pos).kind == TokenKind::StringLit {
                    let tok = tl.get(pos);
                    let item = &mut self.data_store[self.data_count];
                    item.is_string = true;
                    item.str_len = tok.str_len.min(63);
                    item.str_buf[..item.str_len].copy_from_slice(&tok.str_buf[..item.str_len]);
                    self.data_count += 1;
                    pos += 1;
                } else if tl.get(pos).kind == TokenKind::Minus {
                    pos += 1;
                    if tl.get(pos).kind == TokenKind::Number {
                        self.data_store[self.data_count].is_string = false;
                        self.data_store[self.data_count].num_val = -tl.get(pos).num_val;
                        self.data_count += 1;
                        pos += 1;
                    }
                } else if tl.get(pos).kind == TokenKind::Number {
                    self.data_store[self.data_count].is_string = false;
                    self.data_store[self.data_count].num_val = tl.get(pos).num_val;
                    self.data_count += 1;
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
        }
    }

    pub fn run(&mut self, con: &mut Console) {
        self.vars = [0.0; 26];
        self.for_sp = 0;
        self.gosub_sp = 0;
        self.array_count = 0;
        self.string_count = 0;
        self.collect_data();
        self.running = true;
        self.pc = 0;

        while self.running && self.pc < self.line_count {
            if crate::interrupts::keybuf_try_read() == Some(27) {
                con.print("\nBREAK IN ");
                print_f64(con, self.lines[self.pc].number as f64);
                con.putchar(b'\n');
                self.running = false;
                break;
            }

            let idx = self.pc;
            self.pc = idx + 1;

            let mut tl = TokenLine::new();
            let line = &self.lines[idx];
            if tokenize(&line.text, line.len, &mut tl).is_err() {
                self.print_error(con, "SYNTAX ERROR", Some(idx));
                break;
            }

            if let Err(e) = self.exec_statement(&tl, 0, con) {
                self.print_error(con, e, Some(idx));
                break;
            }
        }
        self.running = false;
    }

    fn print_error(&mut self, con: &mut Console, msg: &str, line_idx: Option<usize>) {
        con.print("\n? ");
        con.print(msg);
        if let Some(idx) = line_idx {
            if idx < self.line_count {
                con.print(" IN ");
                print_f64(con, self.lines[idx].number as f64);
            }
        }
        con.putchar(b'\n');
        self.running = false;
    }

    pub fn exec_statement(
        &mut self,
        tl: &TokenLine,
        start: usize,
        con: &mut Console,
    ) -> Result<(), &'static str> {
        if tl.count == 0 || tl.get(start).kind == TokenKind::Eol {
            return Ok(());
        }

        match tl.get(start).kind {
            TokenKind::Print => self.exec_print(tl, start + 1, con),
            TokenKind::Let => self.exec_let(tl, start + 1),
            TokenKind::If => self.exec_if(tl, start + 1, con),
            TokenKind::For => self.exec_for(tl, start + 1),
            TokenKind::Next => self.exec_next(tl, start + 1),
            TokenKind::Goto => self.exec_goto(tl, start + 1),
            TokenKind::Gosub => self.exec_gosub(tl, start + 1),
            TokenKind::Return => self.exec_return(),
            TokenKind::Input => self.exec_input(tl, start + 1, con),
            TokenKind::Rem => Ok(()),
            TokenKind::End => { self.running = false; Ok(()) }
            TokenKind::Run => { self.run(con); Ok(()) }
            TokenKind::List => { self.list(con); Ok(()) }
            TokenKind::Clr => { self.clear(); Ok(()) }
            TokenKind::Save => self.exec_save(tl, start + 1, con),
            TokenKind::Load => self.exec_load(tl, start + 1, con),
            TokenKind::Dir => { crate::fs::fs_list(con); Ok(()) }
            TokenKind::Delete => self.exec_delete(tl, start + 1, con),
            TokenKind::Format => self.exec_format(con),
            TokenKind::GrCmd => self.exec_graphics(tl, start + 1, con),
            TokenKind::Plot => self.exec_plot(tl, start + 1),
            TokenKind::Drawto => self.exec_drawto(tl, start + 1),
            TokenKind::Fillto => self.exec_fillto(tl, start + 1),
            TokenKind::ColorCmd => self.exec_color(tl, start + 1),
            TokenKind::Pos => self.exec_pos(tl, start + 1),
            TokenKind::TextCmd => self.exec_text(tl, start + 1),
            TokenKind::Show => { Self::gfx().present(); Ok(()) }
            TokenKind::Sound => self.exec_sound(tl, start + 1),
            TokenKind::Quit => {
                con.print(" SHUTTING DOWN...\n");
                crate::shutdown();
            }
            TokenKind::Dim => self.exec_dim(tl, start + 1),
            TokenKind::On => self.exec_on(tl, start + 1),
            TokenKind::Data => Ok(()), // DATA is collected at RUN start, skip during execution
            TokenKind::Read => self.exec_read(tl, start + 1),
            TokenKind::Restore => { self.data_ptr = 0; Ok(()) }
            TokenKind::Poke => self.exec_poke(tl, start + 1),
            TokenKind::Pause => {
                crate::interrupts::keybuf_read_blocking();
                Ok(())
            }
            TokenKind::Delay => self.exec_delay(tl, start + 1),
            TokenKind::Dos => { self.exec_dos(con); Ok(()) }
            // Bare assignment: A = 5
            TokenKind::Ident if tl.get(start + 1).kind == TokenKind::Eq => {
                self.exec_let(tl, start)
            }
            // Array assignment: A(i) = expr
            TokenKind::Ident if tl.get(start + 1).kind == TokenKind::LParen => {
                self.exec_array_let(tl, start)
            }
            // String assignment: A$ = "hello"
            TokenKind::StrIdent if tl.get(start + 1).kind == TokenKind::Eq => {
                self.exec_string_let(tl, start)
            }
            _ => Err("SYNTAX ERROR"),
        }
    }

    fn exec_print(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let mut pos = start;
        let mut suppress_newline = false;

        while tl.get(pos).kind != TokenKind::Eol {
            suppress_newline = false;

            match tl.get(pos).kind {
                TokenKind::StringLit => {
                    print_token_str(con, tl.get(pos));
                    pos += 1;
                }
                TokenKind::StrIdent => {
                    let idx = var_index(tl.get(pos))?;
                    let s = self.string_get(idx);
                    for &b in s { con.putchar(b); }
                    pos += 1;
                }
                TokenKind::Semicolon | TokenKind::Comma => {
                    suppress_newline = true;
                    pos += 1;
                }
                _ => {
                    let mut parser = Parser::new(tl,pos, &mut self.vars, self as *const _);
                    let val = parser.parse_condition()?;
                    pos = parser.pos;
                    print_f64(con, val);
                }
            }
        }

        if !suppress_newline {
            con.putchar(b'\n');
        }
        Ok(())
    }

    fn exec_let(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let idx = var_index(tl.get(start))?;
        if tl.get(start + 1).kind != TokenKind::Eq {
            return Err("SYNTAX ERROR");
        }
        let mut parser = Parser::new(tl,start + 2, &mut self.vars, self as *const _);
        let val = parser.parse_condition()?;
        self.vars[idx] = val;
        Ok(())
    }

    fn exec_if(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let cond = parser.parse_condition()?;
        let pos = parser.pos;

        if tl.get(pos).kind != TokenKind::Then {
            return Err("SYNTAX ERROR");
        }

        if cond != 0.0 {
            // THEN followed by line number = implicit GOTO
            if tl.get(pos + 1).kind == TokenKind::Number {
                let linenum = tl.get(pos + 1).num_val as u16;
                return self.jump_to_line(linenum);
            }
            self.exec_statement(tl, pos + 1, con)?;
        }
        Ok(())
    }

    fn exec_for(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let vi = var_index(tl.get(start))?;
        if tl.get(start + 1).kind != TokenKind::Eq {
            return Err("SYNTAX ERROR");
        }

        let mut parser = Parser::new(tl,start + 2, &mut self.vars, self as *const _);
        let start_val = parser.parse_expr()?;
        let mut pos = parser.pos;

        if tl.get(pos).kind != TokenKind::To {
            return Err("SYNTAX ERROR");
        }
        pos += 1;

        let mut parser = Parser::new(tl,pos, &mut self.vars, self as *const _);
        let limit = parser.parse_expr()?;
        pos = parser.pos;

        let step = if tl.get(pos).kind == TokenKind::Step {
            let mut parser = Parser::new(tl,pos + 1, &mut self.vars, self as *const _);
            parser.parse_expr()?
        } else {
            1.0
        };

        self.vars[vi] = start_val;

        if self.for_sp >= self.for_stack.len() {
            return Err("FOR OVERFLOW");
        }
        self.for_stack[self.for_sp] = ForFrame {
            var_idx: vi, limit, step, line_idx: self.pc,
        };
        self.for_sp += 1;
        Ok(())
    }

    fn exec_next(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let var_idx = if tl.get(start).kind == TokenKind::Ident && tl.get(start).str_len == 1 {
            Some(var_index(tl.get(start))?)
        } else {
            None
        };

        if self.for_sp == 0 {
            return Err("NEXT WITHOUT FOR");
        }

        let frame_idx = if let Some(vi) = var_idx {
            let mut found = None;
            for i in (0..self.for_sp).rev() {
                if self.for_stack[i].var_idx == vi {
                    found = Some(i);
                    break;
                }
            }
            found.ok_or("NEXT WITHOUT FOR")?
        } else {
            self.for_sp - 1
        };

        let frame = self.for_stack[frame_idx];
        self.vars[frame.var_idx] += frame.step;

        let done = if frame.step > 0.0 {
            self.vars[frame.var_idx] > frame.limit
        } else {
            self.vars[frame.var_idx] < frame.limit
        };

        if done {
            self.for_sp = frame_idx;
        } else {
            self.pc = frame.line_idx;
        }
        Ok(())
    }

    fn exec_goto(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let linenum = parser.parse_expr()? as u16;
        self.jump_to_line(linenum)
    }

    fn exec_gosub(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        if self.gosub_sp >= self.gosub_stack.len() {
            return Err("GOSUB OVERFLOW");
        }
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let linenum = parser.parse_expr()? as u16;
        self.gosub_stack[self.gosub_sp] = self.pc;
        self.gosub_sp += 1;
        self.jump_to_line(linenum)
    }

    fn exec_return(&mut self) -> Result<(), &'static str> {
        if self.gosub_sp == 0 {
            return Err("RETURN WITHOUT GOSUB");
        }
        self.gosub_sp -= 1;
        self.pc = self.gosub_stack[self.gosub_sp];
        Ok(())
    }

    fn exec_input(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let mut pos = start;

        if tl.get(pos).kind == TokenKind::StringLit {
            print_token_str(con, tl.get(pos));
            pos += 1;
            if tl.get(pos).kind == TokenKind::Comma || tl.get(pos).kind == TokenKind::Semicolon {
                pos += 1;
            }
        } else {
            con.print("? ");
        }

        let idx = var_index(tl.get(pos))?;

        let mut buf = [0u8; 80];
        let len = super::read_line(con, &mut buf);
        let (val, _) = super::value::parse_f64_bytes(&buf[..len]);
        self.vars[idx] = val;
        Ok(())
    }

    // --- Disk commands ---

    /// Serialize program to "linenum text\n" format into a static buffer.
    fn serialize_program(&self, buf: &mut [u8]) -> usize {
        let mut pos = 0;
        let mut num_buf = [0u8; 20];
        for i in 0..self.line_count {
            let line = &self.lines[i];
            let n = format_u64(line.number as u64, &mut num_buf);
            for j in 0..n {
                if pos < buf.len() { buf[pos] = num_buf[j]; pos += 1; }
            }
            // Space
            if pos < buf.len() { buf[pos] = b' '; pos += 1; }
            // Text
            for j in 0..line.len {
                if pos < buf.len() { buf[pos] = line.text[j]; pos += 1; }
            }
            // Newline
            if pos < buf.len() { buf[pos] = b'\n'; pos += 1; }
        }
        pos
    }

    /// Deserialize "linenum text\n" format back into program lines.
    fn deserialize_program(&mut self, data: &[u8], len: usize) {
        self.line_count = 0;

        let mut i = 0;
        while i < len {
            // Skip whitespace/newlines
            while i < len && (data[i] == b'\n' || data[i] == b'\r' || data[i] == b' ') {
                i += 1;
            }
            if i >= len { break; }

            // Parse line number
            let mut linenum: u16 = 0;
            while i < len && data[i].is_ascii_digit() {
                linenum = linenum * 10 + (data[i] - b'0') as u16;
                i += 1;
            }
            // Skip space after line number
            while i < len && data[i] == b' ' {
                i += 1;
            }
            // Collect text until newline
            let text_start = i;
            while i < len && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            let text_len = i - text_start;
            if linenum > 0 && text_len > 0 {
                self.insert_line(linenum, &data[text_start..text_start + text_len]);
            }
        }
    }

    fn get_filename(&self, tl: &TokenLine, start: usize) -> Result<([u8; 32], usize), &'static str> {
        let tok = tl.get(start);
        if tok.kind == TokenKind::StringLit || tok.kind == TokenKind::Ident {
            let mut name = [0u8; 32];
            let len = tok.str_len.min(31);
            name[..len].copy_from_slice(&tok.str_buf[..len]);
            Ok((name, len))
        } else {
            Err("SYNTAX ERROR")
        }
    }

    fn disk_buf() -> &'static mut [u8; PROGRAM_BUF_SIZE] {
        static mut BUF: [u8; PROGRAM_BUF_SIZE] = [0; PROGRAM_BUF_SIZE];
        unsafe { &mut *core::ptr::addr_of_mut!(BUF) }
    }

    fn exec_save(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let (name, name_len) = self.get_filename(tl, start)?;
        let buf = Self::disk_buf();
        let size = self.serialize_program(buf);
        crate::fs::fs_save(&name[..name_len], &buf[..size])?;
        con.print(" SAVED\n");
        Ok(())
    }

    fn exec_load(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let (name, name_len) = self.get_filename(tl, start)?;
        let buf = Self::disk_buf();
        let size = crate::fs::fs_load(&name[..name_len], buf, PROGRAM_BUF_SIZE)?;
        self.vars = [0.0; 26];
        self.deserialize_program(buf, size);
        con.print(" LOADED\n");
        Ok(())
    }

    fn exec_delete(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let (name, name_len) = self.get_filename(tl, start)?;
        crate::fs::fs_delete(&name[..name_len])?;
        con.print(" DELETED\n");
        Ok(())
    }

    fn exec_format(&mut self, con: &mut Console) -> Result<(), &'static str> {
        con.print(" FORMAT DISK - ARE YOU SURE? (Y/N) ");
        let mut buf = [0u8; 4];
        let len = super::read_line(con, &mut buf);
        if len > 0 && (buf[0] == b'Y' || buf[0] == b'y') {
            crate::fs::fs_format()?;
            con.print(" FORMATTED\n");
        }
        Ok(())
    }

    fn gfx() -> &'static mut crate::graphics::Graphics {
        unsafe { crate::graphics::GRAPHICS.get() }
    }

    fn parse_xy(&mut self, tl: &TokenLine, start: usize) -> Result<(i32, i32, usize), &'static str> {
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let x = parser.parse_expr()? as i32;
        let pos = parser.pos;
        if tl.get(pos).kind != TokenKind::Comma {
            return Err("SYNTAX ERROR");
        }
        let mut parser = Parser::new(tl,pos + 1, &mut self.vars, self as *const _);
        let y = parser.parse_expr()? as i32;
        Ok((x, y, parser.pos))
    }

    fn exec_graphics(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let mode = parser.parse_expr()? as u8;
        Self::gfx().set_mode(mode, con);
        Ok(())
    }

    fn exec_plot(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let (x, y, _) = self.parse_xy(tl, start)?;
        Self::gfx().plot(x, y);
        Ok(())
    }

    fn exec_drawto(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let (x, y, _) = self.parse_xy(tl, start)?;
        Self::gfx().drawto(x, y);
        Ok(())
    }

    fn exec_fillto(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let (x, y, _) = self.parse_xy(tl, start)?;
        Self::gfx().fillto(x, y);
        Ok(())
    }

    fn exec_color(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let idx = parser.parse_expr()? as u8;
        Self::gfx().set_color(idx);
        Ok(())
    }

    fn exec_pos(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let (x, y, _) = self.parse_xy(tl, start)?;
        Self::gfx().pos(x, y);
        Ok(())
    }

    fn exec_text(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let tok = tl.get(start);
        if tok.kind == TokenKind::StringLit {
            Self::gfx().text(&tok.str_buf[..tok.str_len]);
        } else {
            let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
            let val = parser.parse_expr()?;
            let mut buf = [0u8; 32];
            let len = super::value::format_f64(val, &mut buf);
            Self::gfx().text(&buf[..len]);
        }
        Ok(())
    }

    /// Parse the next comma-separated expression argument.
    fn parse_comma_arg(&mut self, tl: &TokenLine, pos: usize) -> Result<(f64, usize), &'static str> {
        if tl.get(pos).kind != TokenKind::Comma {
            return Err("SYNTAX ERROR");
        }
        let mut parser = Parser::new(tl,pos + 1, &mut self.vars, self as *const _);
        let val = parser.parse_expr()?;
        Ok((val, parser.pos))
    }

    fn exec_sound(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        // SOUND voice, pitch, distortion, volume
        // Voice and distortion are parsed but ignored (PC speaker only)
        let mut parser = Parser::new(tl,start, &mut self.vars, self as *const _);
        let _voice = parser.parse_expr()?;
        let pos = parser.pos;
        let (pitch, pos) = self.parse_comma_arg(tl, pos)?;
        let (_distortion, pos) = self.parse_comma_arg(tl, pos)?;
        let (volume, _) = self.parse_comma_arg(tl, pos)?;

        if volume as i32 == 0 {
            crate::speaker::speaker_off();
        } else {
            let freq = crate::speaker::atari_pitch_to_hz(pitch as u8);
            crate::speaker::speaker_on(freq);
        }
        Ok(())
    }

    fn exec_dim(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let tok = tl.get(start);
        if tok.kind == TokenKind::StrIdent {
            // DIM A$(size)
            let idx = var_index(tok)?;
            let (size, _) = self.parse_comma_arg_or_first(tl, start + 1)?;
            // Check for duplicate
            for s in 0..self.string_count {
                if self.strings[s].name == idx as u8 { return Err("REDIM ERROR"); }
            }
            if self.string_count >= MAX_STRINGS { return Err("TOO MANY STRINGS"); }
            self.strings[self.string_count] = StringVar {
                name: idx as u8, dimmed: true,
                buf: [0; MAX_STR_LEN], len: 0,
            };
            self.string_count += 1;
            let _ = size;
            Ok(())
        } else if tok.kind == TokenKind::Ident {
            // DIM A(dim1) or DIM A(dim1, dim2)
            let idx = var_index(tok)?;
            if tl.get(start + 1).kind != TokenKind::LParen { return Err("SYNTAX ERROR"); }
            let mut parser = Parser::new(tl, start + 2, &mut self.vars, self as *const _);
            let dim1 = parser.parse_expr()? as u16;
            let dim2 = if tl.get(parser.pos).kind == TokenKind::Comma {
                let mut p2 = Parser::new(tl, parser.pos + 1, &mut self.vars, self as *const _);
                let d = p2.parse_expr()? as u16;
                parser.pos = p2.pos;
                d
            } else {
                0
            };
            if tl.get(parser.pos).kind != TokenKind::RParen { return Err("SYNTAX ERROR"); }
            let total = if dim2 > 0 { dim1 as usize * dim2 as usize } else { dim1 as usize };
            if total > MAX_ARRAY_ELEMS { return Err("ARRAY TOO LARGE"); }
            if self.array_find(idx).is_some() { return Err("REDIM ERROR"); }
            if self.array_count >= MAX_ARRAYS { return Err("TOO MANY ARRAYS"); }
            self.arrays[self.array_count] = Array {
                name: idx as u8, dim1, dim2, vals: [0.0; MAX_ARRAY_ELEMS],
            };
            self.array_count += 1;
            Ok(())
        } else {
            Err("SYNTAX ERROR")
        }
    }

    fn parse_comma_arg_or_first(&mut self, tl: &TokenLine, start: usize) -> Result<(f64, usize), &'static str> {
        // Handle optional ( before value
        let pos = if tl.get(start).kind == TokenKind::LParen { start + 1 } else { start };
        let mut parser = Parser::new(tl, pos, &mut self.vars, self as *const _);
        let val = parser.parse_expr()?;
        let end = if tl.get(parser.pos).kind == TokenKind::RParen { parser.pos + 1 } else { parser.pos };
        Ok((val, end))
    }

    fn exec_array_let(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let idx = var_index(tl.get(start))?;
        if tl.get(start + 1).kind != TokenKind::LParen { return Err("SYNTAX ERROR"); }
        let mut parser = Parser::new(tl, start + 2, &mut self.vars, self as *const _);
        let i1 = parser.parse_expr()? as usize;
        let mut pos = parser.pos;
        let i2 = if tl.get(pos).kind == TokenKind::Comma {
            let mut p2 = Parser::new(tl, pos + 1, &mut self.vars, self as *const _);
            let v = p2.parse_expr()? as usize;
            pos = p2.pos;
            v
        } else {
            0
        };
        if tl.get(pos).kind != TokenKind::RParen { return Err("SYNTAX ERROR"); }
        pos += 1;
        if tl.get(pos).kind != TokenKind::Eq { return Err("SYNTAX ERROR"); }
        let mut parser = Parser::new(tl, pos + 1, &mut self.vars, self as *const _);
        let val = parser.parse_condition()?;
        self.array_set(idx, i1, i2, val)
    }

    fn exec_string_let(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let idx = var_index(tl.get(start))?;
        if tl.get(start + 1).kind != TokenKind::Eq { return Err("SYNTAX ERROR"); }
        let tok = tl.get(start + 2);
        if tok.kind == TokenKind::StringLit {
            self.string_set(idx, &tok.str_buf[..tok.str_len])
        } else {
            Err("SYNTAX ERROR")
        }
    }

    fn exec_on(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl, start, &mut self.vars, self as *const _);
        let val = parser.parse_expr()? as i32;
        let mut pos = parser.pos;

        let is_gosub = match tl.get(pos).kind {
            TokenKind::Goto => false,
            TokenKind::Gosub => true,
            _ => return Err("SYNTAX ERROR"),
        };
        pos += 1;

        let mut n = 0i32;
        let mut target: Option<u16> = None;
        while tl.get(pos).kind != TokenKind::Eol {
            if tl.get(pos).kind == TokenKind::Comma { pos += 1; continue; }
            let mut p = Parser::new(tl, pos, &mut self.vars, self as *const _);
            let linenum = p.parse_expr()? as u16;
            pos = p.pos;
            n += 1;
            if n == val { target = Some(linenum); }
        }

        if let Some(linenum) = target {
            if is_gosub {
                if self.gosub_sp >= self.gosub_stack.len() { return Err("GOSUB OVERFLOW"); }
                self.gosub_stack[self.gosub_sp] = self.pc;
                self.gosub_sp += 1;
            }
            self.jump_to_line(linenum)?;
        }
        Ok(())
    }

    fn exec_read(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut pos = start;
        while tl.get(pos).kind != TokenKind::Eol {
            if tl.get(pos).kind == TokenKind::Comma { pos += 1; continue; }
            if self.data_ptr >= self.data_count { return Err("OUT OF DATA"); }

            if tl.get(pos).kind == TokenKind::StrIdent {
                let idx = var_index(tl.get(pos))?;
                let item = &self.data_store[self.data_ptr];
                if !item.is_string { return Err("TYPE MISMATCH"); }
                // Copy to local buf to avoid borrowing self.data_store and self.strings simultaneously
                let len = item.str_len;
                let mut buf = [0u8; 64];
                buf[..len].copy_from_slice(&item.str_buf[..len]);
                self.data_ptr += 1;
                self.string_set(idx, &buf[..len])?;
            } else if tl.get(pos).kind == TokenKind::Ident {
                let idx = var_index(tl.get(pos))?;
                let item = &self.data_store[self.data_ptr];
                if item.is_string { return Err("TYPE MISMATCH"); }
                self.vars[idx] = item.num_val;
                self.data_ptr += 1;
            } else {
                return Err("SYNTAX ERROR");
            }
            pos += 1;
        }
        Ok(())
    }

    fn exec_poke(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl, start, &mut self.vars, self as *const _);
        let addr = parser.parse_expr()? as usize;
        let (val, _) = self.parse_comma_arg(tl, parser.pos)?;
        unsafe { core::ptr::write_volatile(addr as *mut u8, val as u8); }
        Ok(())
    }

    fn exec_delay(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl, start, &mut self.vars, self as *const _);
        let ms = parser.parse_expr()? as u32;
        // PIT runs at 200Hz = 5ms per tick
        let ticks_needed = (ms + 4) / 5; // round up
        let start_ticks = unsafe { core::ptr::read_volatile(&raw const crate::interrupts::TICKS) };
        loop {
            let now = unsafe { core::ptr::read_volatile(&raw const crate::interrupts::TICKS) };
            if now.wrapping_sub(start_ticks) >= ticks_needed as u64 { break; }
            unsafe { core::arch::asm!("hlt"); }
        }
        Ok(())
    }

    fn exec_dos(&mut self, con: &mut Console) {
        loop {
            con.set_color(crate::console::Color::Yellow, crate::console::Color::Black);
            con.print("\n  RUSTARUS DOS\n");
            con.set_color(crate::console::Color::LightCyan, crate::console::Color::Black);
            con.print("  D - Directory\n");
            con.print("  L - Load file\n");
            con.print("  S - Save program\n");
            con.print("  E - Erase file\n");
            con.print("  F - Format disk\n");
            con.print("  B - Back to BASIC\n\n");
            con.set_color(crate::console::Color::White, crate::console::Color::Black);
            con.print("  Choice? ");

            let c = crate::interrupts::keybuf_read_blocking() as u8;
            con.putchar(c);
            con.putchar(b'\n');

            match c.to_ascii_uppercase() {
                b'B' => return,
                b'D' => { crate::fs::fs_list(con); }
                b'L' => {
                    let (name, len) = dos_prompt_filename(con);
                    if len > 0 {
                        let buf = Self::disk_buf();
                        match crate::fs::fs_load(&name[..len], buf, PROGRAM_BUF_SIZE) {
                            Ok(size) => {
                                self.vars = [0.0; 26];
                                self.deserialize_program(buf, size);
                                con.print("  LOADED\n");
                            }
                            Err(e) => { con.print("  "); con.print(e); con.putchar(b'\n'); }
                        }
                    }
                }
                b'S' => {
                    let (name, len) = dos_prompt_filename(con);
                    if len > 0 {
                        let buf = Self::disk_buf();
                        let size = self.serialize_program(buf);
                        match crate::fs::fs_save(&name[..len], &buf[..size]) {
                            Ok(()) => con.print("  SAVED\n"),
                            Err(e) => { con.print("  "); con.print(e); con.putchar(b'\n'); }
                        }
                    }
                }
                b'E' => {
                    let (name, len) = dos_prompt_filename(con);
                    if len > 0 {
                        match crate::fs::fs_delete(&name[..len]) {
                            Ok(()) => con.print("  DELETED\n"),
                            Err(e) => { con.print("  "); con.print(e); con.putchar(b'\n'); }
                        }
                    }
                }
                b'F' => {
                    con.print("  ARE YOU SURE? (Y/N) ");
                    let mut buf = [0u8; 4];
                    let len = super::read_line(con, &mut buf);
                    if len > 0 && (buf[0] == b'Y' || buf[0] == b'y') {
                        match crate::fs::fs_format() {
                            Ok(()) => con.print("  FORMATTED\n"),
                            Err(e) => { con.print("  "); con.print(e); con.putchar(b'\n'); }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn dos_prompt_filename(con: &mut Console) -> ([u8; 32], usize) {
    con.print("  Filename: ");
    let mut name = [0u8; 32];
    let len = super::read_line(con, &mut name);
    (name, len)
}

fn print_token_str(con: &mut Console, tok: &super::token::Token) {
    for i in 0..tok.str_len {
        con.putchar(tok.str_buf[i]);
    }
}

