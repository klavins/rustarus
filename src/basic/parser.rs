// parser.rs - Recursive descent expression parser for BASIC
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

use super::token::{TokenKind, TokenLine, var_index, upper_name};

static mut RNG_STATE: u32 = 0;

/// Seed the RNG. Call once at interpreter startup.
pub fn rng_seed() {
    unsafe {
        let ticks: u32;
        core::arch::asm!("rdtsc", out("eax") ticks, out("edx") _, options(nomem, nostack));
        RNG_STATE = ticks;
    }
}

fn rng_next() -> u32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1103515245).wrapping_add(12345);
        (RNG_STATE >> 16) & 0x7FFF
    }
}

pub struct Parser<'a> {
    tl: &'a TokenLine,
    pub pos: usize,
    vars: *mut [f64; 26],
    state: *const super::exec::BasicState,
}

impl<'a> Parser<'a> {
    pub fn new(tl: &'a TokenLine, pos: usize, vars: *mut [f64; 26], state: *const super::exec::BasicState) -> Self {
        Self { tl, pos, vars, state }
    }

    fn kind(&self) -> TokenKind {
        self.tl.get(self.pos).kind
    }

    fn advance(&mut self) {
        if self.pos < self.tl.count {
            self.pos += 1;
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), &'static str> {
        if self.kind() == kind {
            self.advance();
            Ok(())
        } else {
            Err("SYNTAX ERROR")
        }
    }

    pub fn parse_condition(&mut self) -> Result<f64, &'static str> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<f64, &'static str> {
        let mut val = self.parse_and()?;
        while self.kind() == TokenKind::Or {
            self.advance();
            let rhs = self.parse_and()?;
            val = if val != 0.0 || rhs != 0.0 { 1.0 } else { 0.0 };
        }
        Ok(val)
    }

    fn parse_and(&mut self) -> Result<f64, &'static str> {
        let mut val = self.parse_not()?;
        while self.kind() == TokenKind::And {
            self.advance();
            let rhs = self.parse_not()?;
            val = if val != 0.0 && rhs != 0.0 { 1.0 } else { 0.0 };
        }
        Ok(val)
    }

    fn parse_not(&mut self) -> Result<f64, &'static str> {
        if self.kind() == TokenKind::Not {
            self.advance();
            let val = self.parse_comparison()?;
            return Ok(if val == 0.0 { 1.0 } else { 0.0 });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<f64, &'static str> {
        let lhs = self.parse_expr()?;
        let op = self.kind();
        match op {
            TokenKind::Eq | TokenKind::Lt | TokenKind::Gt |
            TokenKind::Le | TokenKind::Ge | TokenKind::Ne => {
                self.advance();
                let rhs = self.parse_expr()?;
                let result = match op {
                    TokenKind::Eq => (lhs - rhs).abs() < 1e-9,
                    TokenKind::Ne => (lhs - rhs).abs() >= 1e-9,
                    TokenKind::Lt => lhs < rhs,
                    TokenKind::Gt => lhs > rhs,
                    TokenKind::Le => lhs <= rhs,
                    TokenKind::Ge => lhs >= rhs,
                    _ => false,
                };
                Ok(if result { 1.0 } else { 0.0 })
            }
            _ => Ok(lhs),
        }
    }

    pub fn parse_expr(&mut self) -> Result<f64, &'static str> {
        let mut val = self.parse_term()?;
        loop {
            match self.kind() {
                TokenKind::Plus => { self.advance(); val += self.parse_term()?; }
                TokenKind::Minus => { self.advance(); val -= self.parse_term()?; }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_term(&mut self) -> Result<f64, &'static str> {
        let mut val = self.parse_unary()?;
        loop {
            match self.kind() {
                TokenKind::Star => { self.advance(); val *= self.parse_unary()?; }
                TokenKind::Slash => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    if rhs == 0.0 { return Err("DIVISION BY ZERO"); }
                    val /= rhs;
                }
                TokenKind::Percent => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    if rhs == 0.0 { return Err("DIVISION BY ZERO"); }
                    val %= rhs;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_unary(&mut self) -> Result<f64, &'static str> {
        if self.kind() == TokenKind::Minus {
            self.advance();
            let val = self.parse_atom()?;
            return Ok(-val);
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<f64, &'static str> {
        match self.kind() {
            TokenKind::Number => {
                let val = self.tl.get(self.pos).num_val;
                self.advance();
                Ok(val)
            }
            TokenKind::LParen => {
                self.advance();
                let val = self.parse_condition()?;
                self.expect(TokenKind::RParen)?;
                Ok(val)
            }
            TokenKind::Ident => {
                let tok = self.tl.get(self.pos);

                // LEN(A$) — string length function
                if tok.str_len == 3 && tok.str_buf[0].to_ascii_uppercase() == b'L'
                    && tok.str_buf[1].to_ascii_uppercase() == b'E'
                    && tok.str_buf[2].to_ascii_uppercase() == b'N'
                {
                    self.advance();
                    self.expect(TokenKind::LParen)?;
                    if self.kind() == TokenKind::StrIdent {
                        let stok = self.tl.get(self.pos);
                        let idx = var_index(stok)?;
                        self.advance();
                        self.expect(TokenKind::RParen)?;
                        let state = unsafe { &*self.state };
                        return Ok(state.string_get(idx).len() as f64);
                    }
                    return Err("TYPE MISMATCH");
                }

                // Check for built-in functions (only if followed by parenthesis)
                if self.tl.get(self.pos + 1).kind == TokenKind::LParen {
                    if let Some(f) = match_builtin(&tok.str_buf[..tok.str_len]) {
                        self.advance();
                        self.expect(TokenKind::LParen)?;
                        let arg = self.parse_condition()?;
                        self.expect(TokenKind::RParen)?;
                        return f(arg);
                    }
                }

                // Built-in constants: SCRW, SCRH
                if let Some(val) = match_builtin_const(&tok.str_buf[..tok.str_len]) {
                    self.advance();
                    return Ok(val);
                }

                // Multi-letter named variable (PASS, TEST, SCORE, etc.)
                if tok.str_len > 1 {
                    self.advance();
                    // Check array access for multi-letter names
                    if self.kind() == TokenKind::LParen {
                        let idx = var_index(tok)?;
                        self.advance();
                        let i1 = self.parse_condition()? as usize;
                        let i2 = if self.kind() == TokenKind::Comma {
                            self.advance();
                            self.parse_condition()? as usize
                        } else { 0 };
                        self.expect(TokenKind::RParen)?;
                        let state = unsafe { &*self.state };
                        return state.array_get(idx, i1, i2);
                    }
                    let state = unsafe { &*self.state };
                    let mut upper = [0u8; 16];
                    let len = upper_name(tok, &mut upper);
                    return Ok(state.named_var_get(&upper[..len]));
                }

                // Single-letter variable A-Z
                let idx = var_index(tok)?;
                self.advance();

                // Array access: A(i) or A(i,j)
                if self.kind() == TokenKind::LParen {
                    self.advance();
                    let i1 = self.parse_condition()? as usize;
                    let i2 = if self.kind() == TokenKind::Comma {
                        self.advance();
                        self.parse_condition()? as usize
                    } else { 0 };
                    self.expect(TokenKind::RParen)?;
                    let state = unsafe { &*self.state };
                    return state.array_get(idx, i1, i2);
                }

                Ok(unsafe { (*self.vars)[idx] })
            }
            _ => Err("SYNTAX ERROR"),
        }
    }
}

fn match_builtin(name: &[u8]) -> Option<fn(f64) -> Result<f64, &'static str>> {
    if name.len() < 2 || name.len() > 4 {
        return None;
    }
    let mut upper = [0u8; 4];
    let len = name.len().min(4);
    for i in 0..len {
        upper[i] = name[i].to_ascii_uppercase();
    }
    match (name.len(), &upper[..name.len()]) {
        (3, b"RND") => Some(|n| {
            let r = rng_next();
            if n > 0.0 { Ok((r as f64) % n) } else { Ok(r as f64 / 32768.0) }
        }),
        (3, b"ABS") => Some(|n| Ok(if n < 0.0 { -n } else { n })),
        (3, b"INT") => Some(|n| Ok((n as i64) as f64)),
        (3, b"SQR") => Some(|n| {
            if n < 0.0 { Err("ILLEGAL QUANTITY") } else { Ok(sqrt_approx(n)) }
        }),
        (4, b"PEEK") => Some(|addr| {
            let byte = unsafe { core::ptr::read_volatile(addr as usize as *const u8) };
            Ok(byte as f64)
        }),
        (3, b"SIN") => Some(|n| Ok(sin_approx(n))),
        (3, b"COS") => Some(|n| Ok(cos_approx(n))),
        _ => None,
    }
}

fn match_builtin_const(name: &[u8]) -> Option<f64> {
    if name.len() < 2 || name.len() > 4 {
        return None;
    }
    let mut upper = [0u8; 4];
    let len = name.len().min(4);
    for i in 0..len {
        upper[i] = name[i].to_ascii_uppercase();
    }
    match (name.len(), &upper[..name.len()]) {
        (2, b"PI") => return Some(core::f64::consts::PI),
        (3, b"RND") => {
            let r = rng_next();
            return Some(r as f64 / 32768.0);
        }
        _ => {}
    }
    // SCRW/SCRH need 4-letter match
    if name.len() != 4 { return None; }
    match &upper {
        b"SCRW" => {
            let gfx = unsafe { crate::graphics::GRAPHICS.get() };
            Some(gfx.virt_width() as f64)
        }
        b"SCRH" => {
            let gfx = unsafe { crate::graphics::GRAPHICS.get() };
            Some(gfx.virt_height() as f64)
        }
        _ => None,
    }
}

fn sqrt_approx(val: f64) -> f64 {
    if val == 0.0 { return 0.0; }
    let mut guess = val;
    for _ in 0..10 {
        let next = 0.5 * (guess + val / guess);
        if (next - guess).abs() < 1e-12 { break; }
        guess = next;
    }
    guess
}

/// Taylor series sin approximation (no libm in no_std)
fn sin_approx(x: f64) -> f64 {
    let pi = core::f64::consts::PI;
    let two_pi = 2.0 * pi;
    let mut a = x % two_pi;
    while a > pi { a -= two_pi; }
    while a < -pi { a += two_pi; }
    // Taylor series: x - x^3/6 + x^5/120 - x^7/5040 + x^9/362880
    let x2 = a * a;
    let mut term = a;
    let mut sum = a;
    for i in 1..10 {
        term *= -x2 / ((2 * i) as f64 * (2 * i + 1) as f64);
        sum += term;
    }
    sum
}

fn cos_approx(x: f64) -> f64 {
    sin_approx(x + core::f64::consts::FRAC_PI_2)
}
