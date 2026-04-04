// speaker.rs - PC speaker driver via PIT channel 2
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

use crate::io::{outb, inb};

const PIT_FREQ: u32 = 1_193_182;
const PIT_CMD: u16 = 0x43;
const PIT_CH2_DATA: u16 = 0x42;
const PIT_CH2_MODE: u8 = 0xB6; // channel 2, lo/hi, square wave
const SYS_CTRL: u16 = 0x61;
const SPK_ENABLE: u8 = 0x03; // PIT gate + speaker enable

pub fn speaker_on(freq: u32) {
    if freq == 0 {
        return;
    }
    let div = PIT_FREQ / freq;
    unsafe {
        outb(PIT_CMD, PIT_CH2_MODE);
        outb(PIT_CH2_DATA, (div & 0xFF) as u8);
        outb(PIT_CH2_DATA, ((div >> 8) & 0xFF) as u8);
        let val = inb(SYS_CTRL);
        outb(SYS_CTRL, val | SPK_ENABLE);
    }
}

pub fn speaker_off() {
    unsafe {
        let val = inb(SYS_CTRL);
        outb(SYS_CTRL, val & !SPK_ENABLE);
    }
}

/// Convert Atari BASIC pitch (0-255) to frequency in Hz.
pub fn atari_pitch_to_hz(pitch: u8) -> u32 {
    63920 / (2 * (pitch as u32 + 1))
}
