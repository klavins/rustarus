#![no_std]
#![no_main]

mod console;
mod font;
mod interrupts;

use console::Console;
use core::arch::asm;
use spin::Mutex;
use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;

/// Global console behind a spin lock.
static CONSOLE: Mutex<Console> = Mutex::new(Console::new());

/// Shadow buffer at a fixed address in conventional memory above 32 MB.
const SHADOW_BUF_ADDR: usize = 0x0200_0000;

#[entry]
fn main() -> Status {
    // Locate GOP — find the handle that owns the protocol first
    let gop_handle = match uefi::boot::get_handle_for_protocol::<GraphicsOutput>() {
        Ok(h) => h,
        Err(_) => loop {
            unsafe { asm!("hlt") };
        },
    };
    let mut gop = match uefi::boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle) {
        Ok(g) => g,
        Err(_) => loop {
            unsafe { asm!("hlt") };
        },
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

    // Read framebuffer info before exiting boot services
    let mode_info = gop.current_mode_info();
    let (width, height) = mode_info.resolution();
    let stride = mode_info.stride();
    let fb_base = gop.frame_buffer().as_mut_ptr();
    let pitch = stride * 4;

    drop(gop);

    let _ = unsafe { uefi::boot::exit_boot_services(None) };

    let shadow = SHADOW_BUF_ADDR as *mut u8;

    {
        let mut con = CONSOLE.lock();
        con.init(fb_base, shadow, width as u32, height as u32, pitch as u32);
        con.set_color(2, 0);
        con.print(" UEFI Boot\n");
        con.set_color(11, 0);
        con.print(" Display: ");
        print_u32_locked(&mut con, width as u32);
        con.print("x");
        print_u32_locked(&mut con, height as u32);
        con.print("\n");
    }

    unsafe { interrupts::init() };

    {
        let mut con = CONSOLE.lock();
        con.set_color(7, 0);
        con.print(" RUSTARUS OS v1\n");
        con.set_color(15, 0);
        con.print(" > ");
    }

    // Prompt loop
    let mut line = [0u8; 80];
    let mut pos = 0usize;

    loop {
        let c = interrupts::keybuf_read_blocking() as u8;
        let mut con = CONSOLE.lock();

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

fn print_u32_locked(con: &mut Console, mut n: u32) {
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
    // Try to acquire the lock (may deadlock if panic inside locked section)
    if let Some(mut con) = CONSOLE.try_lock() {
        con.set_color(12, 0);
        con.print("\nPANIC: ");
        if let Some(loc) = info.location() {
            con.print(loc.file());
            con.print(":");
            print_u32_locked(&mut con, loc.line());
        }
        con.print("\n");
    }
    loop {
        unsafe { asm!("hlt") };
    }
}
