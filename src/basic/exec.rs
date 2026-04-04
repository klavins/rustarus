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
    }

    pub fn run(&mut self, con: &mut Console) {
        self.vars = [0.0; 26];
        self.for_sp = 0;
        self.gosub_sp = 0;
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
            TokenKind::Ident if tl.get(start + 1).kind == TokenKind::Eq => {
                self.exec_let(tl, start)
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
                TokenKind::Semicolon | TokenKind::Comma => {
                    suppress_newline = true;
                    pos += 1;
                }
                _ => {
                    let mut parser = Parser::new(tl, pos, &mut self.vars);
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
        let mut parser = Parser::new(tl, start + 2, &mut self.vars);
        let val = parser.parse_condition()?;
        self.vars[idx] = val;
        Ok(())
    }

    fn exec_if(&mut self, tl: &TokenLine, start: usize, con: &mut Console) -> Result<(), &'static str> {
        let mut parser = Parser::new(tl, start, &mut self.vars);
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

        let mut parser = Parser::new(tl, start + 2, &mut self.vars);
        let start_val = parser.parse_expr()?;
        let mut pos = parser.pos;

        if tl.get(pos).kind != TokenKind::To {
            return Err("SYNTAX ERROR");
        }
        pos += 1;

        let mut parser = Parser::new(tl, pos, &mut self.vars);
        let limit = parser.parse_expr()?;
        pos = parser.pos;

        let step = if tl.get(pos).kind == TokenKind::Step {
            let mut parser = Parser::new(tl, pos + 1, &mut self.vars);
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
        let mut parser = Parser::new(tl, start, &mut self.vars);
        let linenum = parser.parse_expr()? as u16;
        self.jump_to_line(linenum)
    }

    fn exec_gosub(&mut self, tl: &TokenLine, start: usize) -> Result<(), &'static str> {
        if self.gosub_sp >= self.gosub_stack.len() {
            return Err("GOSUB OVERFLOW");
        }
        let mut parser = Parser::new(tl, start, &mut self.vars);
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
}

fn print_token_str(con: &mut Console, tok: &super::token::Token) {
    for i in 0..tok.str_len {
        con.putchar(tok.str_buf[i]);
    }
}

