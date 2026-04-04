// value.rs - f64 formatting and number parsing for no_std BASIC
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

/// Print an f64 value to the console.
/// Integers print without decimal point. Non-integers get up to 6 decimal places.
pub fn print_f64(con: &mut Console, val: f64) {
    let mut buf = [0u8; 32];
    let len = format_f64(val, &mut buf);
    for i in 0..len {
        con.putchar(buf[i]);
    }
}

/// Format f64 into a byte buffer. Returns number of bytes written.
pub fn format_f64(val: f64, buf: &mut [u8; 32]) -> usize {
    if val != val {
        // NaN
        buf[0] = b'N'; buf[1] = b'a'; buf[2] = b'N';
        return 3;
    }

    let mut pos = 0;

    if val < 0.0 {
        buf[pos] = b'-';
        pos += 1;
        return pos + format_positive(-val, &mut buf[pos..]);
    }

    if val == 0.0 {
        buf[0] = b'0';
        return 1;
    }

    format_positive(val, buf)
}

fn format_positive(val: f64, buf: &mut [u8]) -> usize {
    // Check if it's an integer (within f64 precision)
    let int_part = val as i64;
    if (int_part as f64 - val).abs() < 1e-9 && val < 1e15 {
        return format_i64(int_part, buf);
    }

    // Non-integer: print with up to 6 decimal places
    let int_part = val as u64;
    let mut pos = format_u64(int_part, buf);
    buf[pos] = b'.';
    pos += 1;

    let mut frac = val - int_part as f64;
    let mut digits = 0;
    let max_digits = 6;

    while digits < max_digits {
        frac *= 10.0;
        let d = frac as u8;
        buf[pos] = b'0' + d;
        pos += 1;
        frac -= d as f64;
        digits += 1;
    }

    // Strip trailing zeros
    while pos > 0 && buf[pos - 1] == b'0' {
        pos -= 1;
    }
    // Strip trailing dot
    if pos > 0 && buf[pos - 1] == b'.' {
        pos -= 1;
    }

    pos
}

fn format_i64(val: i64, buf: &mut [u8]) -> usize {
    format_u64(val as u64, buf)
}

pub fn format_u64(mut val: u64, buf: &mut [u8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut i = 0;
    while val > 0 {
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}

/// Parse a decimal number from a byte slice. Returns (value, bytes_consumed).
pub fn parse_f64_bytes(input: &[u8]) -> (f64, usize) {
    let mut i = 0;
    let mut val: f64 = 0.0;
    let neg = if i < input.len() && input[i] == b'-' {
        i += 1;
        true
    } else {
        false
    };

    while i < input.len() && input[i].is_ascii_digit() {
        val = val * 10.0 + (input[i] - b'0') as f64;
        i += 1;
    }
    if i < input.len() && input[i] == b'.' {
        i += 1;
        let mut frac = 0.1;
        while i < input.len() && input[i].is_ascii_digit() {
            val += (input[i] - b'0') as f64 * frac;
            frac *= 0.1;
            i += 1;
        }
    }

    if neg { val = -val; }
    (val, i)
}
