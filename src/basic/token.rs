// token.rs - BASIC tokenizer with keyword recognition
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

pub const MAX_TOKENS: usize = 32;
pub const MAX_TOKEN_LEN: usize = 64;

#[derive(Copy, Clone, PartialEq)]
pub enum TokenKind {
    Number,
    StringLit,
    Ident,
    Print,
    Let,
    If,
    Then,
    For,
    To,
    Step,
    Next,
    Goto,
    Gosub,
    Return,
    Input,
    Rem,
    Run,
    List,
    Clr,
    End,
    Save,
    Load,
    Dir,
    Delete,
    Format,
    GrCmd,
    Plot,
    Drawto,
    Fillto,
    ColorCmd,
    Pos,
    TextCmd,
    Show,
    Sound,
    Quit,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Lt,
    Gt,
    Le,
    Ge,
    Ne,
    And,
    Or,
    Not,
    Comma,
    Semicolon,
    LParen,
    RParen,
    Eol,
}

#[derive(Copy, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub num_val: f64,
    pub str_buf: [u8; MAX_TOKEN_LEN],
    pub str_len: usize,
}

impl Token {
    pub const fn empty() -> Self {
        Self {
            kind: TokenKind::Eol,
            num_val: 0.0,
            str_buf: [0; MAX_TOKEN_LEN],
            str_len: 0,
        }
    }
}

/// Extract A-Z variable index from a single-letter identifier token.
pub fn var_index(tok: &Token) -> Result<usize, &'static str> {
    if tok.kind != TokenKind::Ident || tok.str_len != 1 {
        return Err("SYNTAX ERROR");
    }
    let upper = tok.str_buf[0].to_ascii_uppercase();
    if upper < b'A' || upper > b'Z' {
        return Err("SYNTAX ERROR");
    }
    Ok((upper - b'A') as usize)
}

pub struct TokenLine {
    pub tokens: [Token; MAX_TOKENS],
    pub count: usize,
}

impl TokenLine {
    pub const fn new() -> Self {
        Self {
            tokens: [Token::empty(); MAX_TOKENS],
            count: 0,
        }
    }

    pub fn get(&self, i: usize) -> &Token {
        if i < self.count {
            &self.tokens[i]
        } else {
            &self.tokens[self.count]
        }
    }
}

struct KeywordEntry {
    name: &'static [u8],
    kind: TokenKind,
}

static KEYWORDS: &[KeywordEntry] = &[
    KeywordEntry { name: b"PRINT", kind: TokenKind::Print },
    KeywordEntry { name: b"LET", kind: TokenKind::Let },
    KeywordEntry { name: b"IF", kind: TokenKind::If },
    KeywordEntry { name: b"THEN", kind: TokenKind::Then },
    KeywordEntry { name: b"FOR", kind: TokenKind::For },
    KeywordEntry { name: b"TO", kind: TokenKind::To },
    KeywordEntry { name: b"STEP", kind: TokenKind::Step },
    KeywordEntry { name: b"NEXT", kind: TokenKind::Next },
    KeywordEntry { name: b"GOTO", kind: TokenKind::Goto },
    KeywordEntry { name: b"GOSUB", kind: TokenKind::Gosub },
    KeywordEntry { name: b"RETURN", kind: TokenKind::Return },
    KeywordEntry { name: b"INPUT", kind: TokenKind::Input },
    KeywordEntry { name: b"REM", kind: TokenKind::Rem },
    KeywordEntry { name: b"RUN", kind: TokenKind::Run },
    KeywordEntry { name: b"LIST", kind: TokenKind::List },
    KeywordEntry { name: b"CLR", kind: TokenKind::Clr },
    KeywordEntry { name: b"END", kind: TokenKind::End },
    KeywordEntry { name: b"SAVE", kind: TokenKind::Save },
    KeywordEntry { name: b"LOAD", kind: TokenKind::Load },
    KeywordEntry { name: b"DIR", kind: TokenKind::Dir },
    KeywordEntry { name: b"DELETE", kind: TokenKind::Delete },
    KeywordEntry { name: b"FORMAT", kind: TokenKind::Format },
    KeywordEntry { name: b"GRAPHICS", kind: TokenKind::GrCmd },
    KeywordEntry { name: b"GR", kind: TokenKind::GrCmd },
    KeywordEntry { name: b"PLOT", kind: TokenKind::Plot },
    KeywordEntry { name: b"DRAWTO", kind: TokenKind::Drawto },
    KeywordEntry { name: b"FILLTO", kind: TokenKind::Fillto },
    KeywordEntry { name: b"COLOR", kind: TokenKind::ColorCmd },
    KeywordEntry { name: b"POS", kind: TokenKind::Pos },
    KeywordEntry { name: b"TEXT", kind: TokenKind::TextCmd },
    KeywordEntry { name: b"SHOW", kind: TokenKind::Show },
    KeywordEntry { name: b"SOUND", kind: TokenKind::Sound },
    KeywordEntry { name: b"QUIT", kind: TokenKind::Quit },
    KeywordEntry { name: b"AND", kind: TokenKind::And },
    KeywordEntry { name: b"OR", kind: TokenKind::Or },
    KeywordEntry { name: b"NOT", kind: TokenKind::Not },
];

fn match_keyword(word: &[u8]) -> Option<TokenKind> {
    for kw in KEYWORDS {
        if kw.name.len() == word.len() {
            let mut matches = true;
            for i in 0..word.len() {
                if word[i].to_ascii_uppercase() != kw.name[i] {
                    matches = false;
                    break;
                }
            }
            if matches {
                return Some(kw.kind);
            }
        }
    }
    None
}

pub fn tokenize(line: &[u8], len: usize, out: &mut TokenLine) -> Result<(), &'static str> {
    out.count = 0;
    let mut i = 0;

    while i < len && out.count < MAX_TOKENS - 1 {
        let c = line[i];

        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }

        let tok = &mut out.tokens[out.count];
        *tok = Token::empty();

        if c.is_ascii_digit() || (c == b'.' && i + 1 < len && line[i + 1].is_ascii_digit()) {
            if c == b'0' && i + 1 < len && (line[i + 1] == b'x' || line[i + 1] == b'X') {
                i += 2;
                let mut val: u64 = 0;
                while i < len {
                    let h = line[i];
                    if h.is_ascii_digit() {
                        val = val * 16 + (h - b'0') as u64;
                    } else if h >= b'a' && h <= b'f' {
                        val = val * 16 + (h - b'a' + 10) as u64;
                    } else if h >= b'A' && h <= b'F' {
                        val = val * 16 + (h - b'A' + 10) as u64;
                    } else {
                        break;
                    }
                    i += 1;
                }
                tok.kind = TokenKind::Number;
                tok.num_val = val as f64;
            } else {
                tok.kind = TokenKind::Number;
                let (val, consumed) = super::value::parse_f64_bytes(&line[i..len]);
                tok.num_val = val;
                i += consumed;
            }
            out.count += 1;
            continue;
        }

        if c == b'"' {
            i += 1;
            tok.kind = TokenKind::StringLit;
            tok.str_len = 0;
            while i < len && line[i] != b'"' {
                if tok.str_len < MAX_TOKEN_LEN - 1 {
                    tok.str_buf[tok.str_len] = line[i];
                    tok.str_len += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            out.count += 1;
            continue;
        }

        if c.is_ascii_alphabetic() {
            tok.str_len = 0;
            while i < len && (line[i].is_ascii_alphanumeric() || line[i] == b'_') {
                if tok.str_len < MAX_TOKEN_LEN - 1 {
                    tok.str_buf[tok.str_len] = line[i];
                    tok.str_len += 1;
                }
                i += 1;
            }

            if let Some(kw) = match_keyword(&tok.str_buf[..tok.str_len]) {
                tok.kind = kw;
                if kw == TokenKind::Rem {
                    out.count += 1;
                    break;
                }
            } else {
                tok.kind = TokenKind::Ident;
            }
            out.count += 1;
            continue;
        }

        match c {
            b'+' => { tok.kind = TokenKind::Plus; i += 1; }
            b'-' => { tok.kind = TokenKind::Minus; i += 1; }
            b'*' => { tok.kind = TokenKind::Star; i += 1; }
            b'/' => { tok.kind = TokenKind::Slash; i += 1; }
            b'%' => { tok.kind = TokenKind::Percent; i += 1; }
            b'=' => { tok.kind = TokenKind::Eq; i += 1; }
            b'<' => {
                if i + 1 < len && line[i + 1] == b'=' {
                    tok.kind = TokenKind::Le; i += 2;
                } else if i + 1 < len && line[i + 1] == b'>' {
                    tok.kind = TokenKind::Ne; i += 2;
                } else {
                    tok.kind = TokenKind::Lt; i += 1;
                }
            }
            b'>' => {
                if i + 1 < len && line[i + 1] == b'=' {
                    tok.kind = TokenKind::Ge; i += 2;
                } else {
                    tok.kind = TokenKind::Gt; i += 1;
                }
            }
            b'(' => { tok.kind = TokenKind::LParen; i += 1; }
            b')' => { tok.kind = TokenKind::RParen; i += 1; }
            b',' => { tok.kind = TokenKind::Comma; i += 1; }
            b';' => { tok.kind = TokenKind::Semicolon; i += 1; }
            _ => return Err("SYNTAX ERROR"),
        }
        out.count += 1;
    }

    if out.count < MAX_TOKENS {
        out.tokens[out.count] = Token::empty();
    }

    Ok(())
}
