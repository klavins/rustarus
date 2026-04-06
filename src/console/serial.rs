// serial.rs - COM1 serial port output (0x3F8)
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

use crate::os::io::{outb, inb};

const COM1: u16 = 0x3F8;
static mut PORT_EXISTS: bool = false;

pub fn serial_init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x01);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        // Probe: check if the line status register reads back sensibly
        let lsr = inb(COM1 + 5);
        PORT_EXISTS = lsr != 0xFF;
    }
}

pub fn serial_putchar(c: u8) {
    unsafe {
        if !PORT_EXISTS { return; }
        for _ in 0..10000u32 {
            if inb(COM1 + 5) & 0x20 != 0 { break; }
        }
        outb(COM1, c);
    }
}

pub fn serial_print(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            serial_putchar(b'\r');
        }
        serial_putchar(b);
    }
}
