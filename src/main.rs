// main.rs - UEFI entry point, GOP framebuffer setup, and prompt loop
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

#![no_std]
#![no_main]

mod console;
mod font;
mod interrupts;

use console::{Color, Console, ConsoleCell};
use core::arch::asm;
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;

static CONSOLE: ConsoleCell = ConsoleCell::new(Console::new());

#[inline(always)]
fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

#[entry]
fn main() -> Status {
    let gop_handle = match uefi::boot::get_handle_for_protocol::<GraphicsOutput>() {
        Ok(h) => h,
        Err(_) => halt(),
    };
    let mut gop = match uefi::boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle) {
        Ok(g) => g,
        Err(_) => halt(),
    };

    // Find best mode (max resolution capped at 1920x1200)
    let mut best_mode = None;
    let mut best_pixels = 0u64;
    for mode in gop.modes() {
        let info = mode.info();
        let (w, h) = info.resolution();
        if w > 1920 || h > 1200 {
            continue;
        }
        let pixels = (w as u64) * (h as u64);
        if pixels > best_pixels {
            best_pixels = pixels;
            best_mode = Some(mode);
        }
    }
    if let Some(mode) = best_mode {
        let _ = gop.set_mode(&mode);
    }

    let mode_info = gop.current_mode_info();
    let (width, height) = mode_info.resolution();
    let stride = mode_info.stride();
    let fb_base = gop.frame_buffer().as_mut_ptr();
    let pitch = stride * 4;
    let fb_size = pitch * height;

    drop(gop);

    let mmap = unsafe { uefi::boot::exit_boot_services(None) };

    // Find a free conventional memory region for the shadow buffer
    let fb_start = fb_base as u64;
    let fb_end = fb_start + fb_size as u64;
    let mut best_base: u64 = 0;
    let mut best_size: u64 = 0;

    for desc in mmap.entries() {
        if desc.ty != uefi::mem::memory_map::MemoryType::CONVENTIONAL {
            continue;
        }
        let base = desc.phys_start;
        let size = desc.page_count * 4096;
        if base < 0x10_0000 || base >= 0x1_0000_0000 {
            continue;
        }
        let end = base + size;
        if base < fb_end && end > fb_start {
            continue;
        }
        if size < fb_size as u64 {
            continue;
        }
        if size > best_size {
            best_base = base;
            best_size = size;
        }
    }

    if best_base == 0 {
        halt();
    }

    let shadow = best_base as *mut u8;

    let con = unsafe { CONSOLE.get() };
    con.init(fb_base, shadow, width as u32, height as u32, pitch as u32);

    con.set_color(Color::Green, Color::Black);
    con.print(" UEFI Boot\n");
    con.set_color(Color::LightCyan, Color::Black);
    con.print(" Display: ");
    print_u32(con, width as u32);
    con.print("x");
    print_u32(con, height as u32);
    con.print("\n");

    unsafe { interrupts::init() };

    con.set_color(Color::LightGray, Color::Black);
    con.print(" RUSTARUS OS v1\n");
    con.set_color(Color::White, Color::Black);
    con.print(" > ");

    let mut line = [0u8; 80];
    let mut pos = 0usize;

    loop {
        let c = interrupts::keybuf_read_blocking() as u8;

        if c == b'\n' {
            con.putchar(b'\n');
            if pos > 0 {
                con.print(" You typed: ");
                for i in 0..pos {
                    con.putchar(line[i]);
                }
                con.putchar(b'\n');
            }
            pos = 0;
            con.print(" > ");
        } else if c == b'\x08' {
            if pos > 0 {
                pos -= 1;
                con.putchar(b'\x08');
            }
        } else if c != 0 && pos < line.len() - 1 {
            line[pos] = c;
            pos += 1;
            con.putchar(c);
        }
    }
}

fn print_u32(con: &mut Console, mut n: u32) {
    if n == 0 {
        con.putchar(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        con.putchar(buf[i]);
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let con = unsafe { CONSOLE.get() };
    con.set_color(Color::LightRed, Color::Black);
    con.print("\nPANIC: ");
    if let Some(loc) = info.location() {
        con.print(loc.file());
        con.print(":");
        print_u32(con, loc.line());
    }
    con.print("\n");
    halt();
}
