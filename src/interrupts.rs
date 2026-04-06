// interrupts.rs - GDT, IDT, PIC remapping, PIT timer, and PS/2 keyboard handler
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

use core::arch::{asm, naked_asm};
use crate::io::{outb, inb};

// PIC ports
const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;
const PIC_EOI: u8 = 0x20;

// PS/2 keyboard
const KB_DATA_PORT: u16 = 0x60;

// Special key codes (128+) for non-ASCII keys
pub const KEY_ARROW_UP: i32 = 128;
pub const KEY_ARROW_DOWN: i32 = 129;
pub const KEY_ARROW_LEFT: i32 = 130;
pub const KEY_ARROW_RIGHT: i32 = 131;
pub const KEY_HOME: i32 = 132;
pub const KEY_END: i32 = 133;
pub const KEY_PAGE_UP: i32 = 134;
pub const KEY_PAGE_DOWN: i32 = 135;
pub const KEY_DELETE: i32 = 136;
pub const KEY_INSERT: i32 = 137;

// PIT
const PIT_HZ: u32 = 200;
const PIT_DIVISOR: u16 = (1193182 / PIT_HZ) as u16;

// Key buffer
const KEYBUF_SIZE: usize = 64;
static mut KEYBUF: [i32; KEYBUF_SIZE] = [0; KEYBUF_SIZE];
static mut KEYBUF_HEAD: usize = 0;
static mut KEYBUF_TAIL: usize = 0;

static mut SHIFT_HELD: bool = false;
static mut CTRL_HELD: bool = false;
static mut E0_PREFIX: bool = false;

/// Key state array — 128 bytes, 1=pressed, 0=released, indexed by scancode.
pub static mut KEYSTATE: [u8; 128] = [0; 128];

/// Pre-computed addresses for ISR access — avoids &raw mut codegen issues in interrupt context.
pub static mut ISR_ADDRS: IsrAddrs = IsrAddrs {
    keystate: 0,
    shift_held: 0,
    ctrl_held: 0,
    e0_prefix: 0,
};

pub struct IsrAddrs {
    pub keystate: usize,
    pub shift_held: usize,
    pub ctrl_held: usize,
    pub e0_prefix: usize,
}

pub static mut TICKS: u64 = 0;

// Scancode tables — 58 entries each (scancodes 0x00..0x39)
#[rustfmt::skip]
static SCANCODE_LOWER: [u8; 58] = [
    0,  27, b'1',b'2',b'3',b'4',b'5',b'6',b'7',b'8',b'9',b'0',b'-',b'=',b'\x08',
    b'\t',b'q',b'w',b'e',b'r',b't',b'y',b'u',b'i',b'o',b'p',b'[',b']',b'\n',
    0,  b'a',b's',b'd',b'f',b'g',b'h',b'j',b'k',b'l',b';',b'\'',b'`',
    0,  b'\\',b'z',b'x',b'c',b'v',b'b',b'n',b'm',b',',b'.',b'/',0,
    b'*',0,  b' ',
];

#[rustfmt::skip]
static SCANCODE_UPPER: [u8; 58] = [
    0,  27, b'!',b'@',b'#',b'$',b'%',b'^',b'&',b'*',b'(',b')',b'_',b'+',b'\x08',
    b'\t',b'Q',b'W',b'E',b'R',b'T',b'Y',b'U',b'I',b'O',b'P',b'{',b'}',b'\n',
    0,  b'A',b'S',b'D',b'F',b'G',b'H',b'J',b'K',b'L',b':',b'"',b'~',
    0,  b'|',b'Z',b'X',b'C',b'V',b'B',b'N',b'M',b'<',b'>',b'?',0,
    b'*',0,  b' ',
];


fn keybuf_put(c: i32) {
    unsafe {
        let head = core::ptr::read_volatile(&raw const KEYBUF_HEAD);
        let tail = core::ptr::read_volatile(&raw const KEYBUF_TAIL);
        let next = (head + 1) % KEYBUF_SIZE;
        if next != tail {
            (*&raw mut KEYBUF)[head] = c;
            core::ptr::write_volatile(&raw mut KEYBUF_HEAD, next);
        }
    }
}

/// Blocking read from key buffer — halts CPU until a key is available.
pub fn keybuf_read_blocking() -> i32 {
    unsafe {
        loop {
            let head = core::ptr::read_volatile(&raw const KEYBUF_HEAD);
            let tail = core::ptr::read_volatile(&raw const KEYBUF_TAIL);
            if head != tail {
                break;
            }
            asm!("hlt");
        }
        let tail = core::ptr::read_volatile(&raw const KEYBUF_TAIL);
        let c = (*&raw const KEYBUF)[tail];
        core::ptr::write_volatile(&raw mut KEYBUF_TAIL, (tail + 1) % KEYBUF_SIZE);
        c
    }
}

/// Non-blocking read from key buffer. Returns Some(key) or None.
pub fn keybuf_try_read() -> Option<i32> {
    unsafe {
        let head = core::ptr::read_volatile(&raw const KEYBUF_HEAD);
        let tail = core::ptr::read_volatile(&raw const KEYBUF_TAIL);
        if head == tail {
            return None;
        }
        let c = (*&raw const KEYBUF)[tail];
        core::ptr::write_volatile(&raw mut KEYBUF_TAIL, (tail + 1) % KEYBUF_SIZE);
        Some(c)
    }
}

/// Get a pointer to a keystate byte, or None if the address isn't in the keystate range.
/// Maps virtual address 0x70000-0x7007F to the KEYSTATE array.
pub fn keystate_ptr(addr: usize) -> Option<*mut u8> {
    if addr >= 0x70000 && addr < 0x70080 {
        let idx = addr - 0x70000;
        unsafe {
            let addrs = &raw const ISR_ADDRS;
            Some(((*addrs).keystate as *mut u8).add(idx))
        }
    } else {
        None
    }
}

/// Flush all pending keys from the buffer.
pub fn keybuf_flush() {
    unsafe {
        let head = core::ptr::read_volatile(&raw const KEYBUF_HEAD);
        core::ptr::write_volatile(&raw mut KEYBUF_TAIL, head);
    }
}

// --- IDT structures (x86_64) ---

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

const EMPTY_IDT: IdtEntry = IdtEntry {
    offset_low: 0, selector: 0, ist: 0, type_attr: 0,
    offset_mid: 0, offset_high: 0, reserved: 0,
};

static mut IDT: [IdtEntry; 256] = [EMPTY_IDT; 256];
static mut IDTP: IdtPtr = IdtPtr { limit: 0, base: 0 };

fn idt_set_gate(n: usize, handler: u64) {
    unsafe {
        let idt = &raw mut IDT;
        let entry = &raw mut (*idt)[n];
        (*entry).offset_low = (handler & 0xFFFF) as u16;
        (*entry).selector = 0x08;
        (*entry).ist = 0;
        (*entry).type_attr = 0x8E;
        (*entry).offset_mid = ((handler >> 16) & 0xFFFF) as u16;
        (*entry).offset_high = ((handler >> 32) & 0xFFFFFFFF) as u32;
        (*entry).reserved = 0;
    }
}

// --- GDT structures ---

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    flags_limit: u8,
    base_high: u8,
}

#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

const EMPTY_GDT: GdtEntry = GdtEntry {
    limit_low: 0, base_low: 0, base_mid: 0,
    access: 0, flags_limit: 0, base_high: 0,
};

static mut GDT: [GdtEntry; 3] = [EMPTY_GDT; 3];
static mut GDTP: GdtPtr = GdtPtr { limit: 0, base: 0 };

unsafe fn gdt_init() {
    unsafe {
        let gdt = &raw mut GDT;
        (*gdt)[0] = EMPTY_GDT;
        (*gdt)[1] = GdtEntry {
            limit_low: 0xFFFF, base_low: 0, base_mid: 0,
            access: 0x9A, flags_limit: 0xAF, base_high: 0,
        };
        (*gdt)[2] = GdtEntry {
            limit_low: 0xFFFF, base_low: 0, base_mid: 0,
            access: 0x92, flags_limit: 0xCF, base_high: 0,
        };

        let gdtp = &raw mut GDTP;
        (*gdtp).limit = (core::mem::size_of::<[GdtEntry; 3]>() - 1) as u16;
        (*gdtp).base = gdt as u64;

        asm!(
            "lgdt [{}]",
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            in(reg) gdtp,
            out("rax") _,
            options(nostack),
        );
    }
}

unsafe fn pic_remap() {
    unsafe {
        outb(PIC1_CMD, 0x11);
        outb(PIC2_CMD, 0x11);
        outb(PIC1_DATA, 0x20);
        outb(PIC2_DATA, 0x28);
        outb(PIC1_DATA, 0x04);
        outb(PIC2_DATA, 0x02);
        outb(PIC1_DATA, 0x01);
        outb(PIC2_DATA, 0x01);
        outb(PIC1_DATA, 0xFC);
        outb(PIC2_DATA, 0xFF);
    }
}

unsafe fn pit_init() {
    unsafe {
        outb(0x43, 0x36);
        outb(0x40, (PIT_DIVISOR & 0xFF) as u8);
        outb(0x40, ((PIT_DIVISOR >> 8) & 0xFF) as u8);
    }
}

// --- Interrupt handlers ---

#[unsafe(no_mangle)]
extern "C" fn timer_handler_inner() {
    unsafe {
        let t = core::ptr::read_volatile(&raw const TICKS);
        core::ptr::write_volatile(&raw mut TICKS, t + 1);
        outb(PIC1_CMD, PIC_EOI);
    }
}

#[unsafe(no_mangle)]
extern "C" fn keyboard_handler_inner() {
    unsafe {
        let scancode = inb(KB_DATA_PORT);

        // Read pre-computed addresses (set once in init, safe to read here)
        let addrs = &raw const ISR_ADDRS;
        let ks_ptr = (*addrs).keystate as *mut u8;
        let shift_ptr = (*addrs).shift_held as *mut bool;
        let ctrl_ptr = (*addrs).ctrl_held as *mut bool;
        let e0_ptr = (*addrs).e0_prefix as *mut bool;

        // Handle E0 prefix for extended scancodes (arrow keys, etc.)
        if scancode == 0xE0 {
            core::ptr::write_volatile(e0_ptr, true);
            outb(PIC1_CMD, PIC_EOI);
            return;
        }
        let is_extended = core::ptr::read_volatile(e0_ptr as *const bool);
        core::ptr::write_volatile(e0_ptr, false);

        // Update keystate array (scancode & 0x7F is always 0..127)
        let sc7 = (scancode & 0x7F) as usize;
        if scancode & 0x80 != 0 {
            core::ptr::write_volatile(ks_ptr.add(sc7), 0);
        } else {
            core::ptr::write_volatile(ks_ptr.add(sc7), 1);
        }

        // Extended keys (preceded by E0)
        if is_extended {
            if scancode & 0x80 == 0 {
                let sc = scancode & 0x7F;
                let key =
                    if sc == 0x48 { KEY_ARROW_UP }
                    else if sc == 0x50 { KEY_ARROW_DOWN }
                    else if sc == 0x4B { KEY_ARROW_LEFT }
                    else if sc == 0x4D { KEY_ARROW_RIGHT }
                    else if sc == 0x47 { KEY_HOME }
                    else if sc == 0x4F { KEY_END }
                    else if sc == 0x49 { KEY_PAGE_UP }
                    else if sc == 0x51 { KEY_PAGE_DOWN }
                    else if sc == 0x53 { KEY_DELETE }
                    else if sc == 0x52 { KEY_INSERT }
                    else { 0 };
                if key != 0 {
                    keybuf_put(key);
                }
            }
            outb(PIC1_CMD, PIC_EOI);
            return;
        }

        // Modifier keys (non-extended)
        match scancode {
            0x2A | 0x36 => core::ptr::write_volatile(shift_ptr, true),
            0xAA | 0xB6 => core::ptr::write_volatile(shift_ptr, false),
            0x1D => core::ptr::write_volatile(ctrl_ptr, true),
            0x9D => core::ptr::write_volatile(ctrl_ptr, false),
            sc if sc & 0x80 == 0 => {
                let idx = sc as usize;
                if idx < SCANCODE_LOWER.len() {
                    let shift = core::ptr::read_volatile(shift_ptr as *const bool);
                    let ctrl = core::ptr::read_volatile(ctrl_ptr as *const bool);
                    let map = if shift { &SCANCODE_UPPER } else { &SCANCODE_LOWER };
                    let mut ascii = map[idx];
                    if ctrl && ascii.is_ascii_alphabetic() {
                        ascii &= 0x1F;
                    }
                    if ascii != 0 {
                        keybuf_put(ascii as i32);
                    }
                }
            }
            _ => {}
        }

        outb(PIC1_CMD, PIC_EOI);
    }
}

// ISR stubs — naked functions that save/restore registers and call inner handlers

// Light ISR stub — GPRs only (for simple handlers like the timer)
macro_rules! isr_stub_light {
    ($name:ident, $handler:literal) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            naked_asm!(
                "cld",
                "push rax", "push rbx", "push rcx", "push rdx",
                "push rbp", "push rsi", "push rdi",
                "push r8", "push r9", "push r10", "push r11",
                "push r12", "push r13", "push r14", "push r15",
                concat!("call ", $handler),
                "pop r15", "pop r14", "pop r13", "pop r12",
                "pop r11", "pop r10", "pop r9", "pop r8",
                "pop rdi", "pop rsi", "pop rbp",
                "pop rdx", "pop rcx", "pop rbx", "pop rax",
                "iretq",
            );
        }
    };
}

// Full ISR stub — GPRs + SSE (for handlers that may trigger SSE codegen)
macro_rules! isr_stub_full {
    ($name:ident, $handler:literal) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            naked_asm!(
                "cld",
                "push rax", "push rbx", "push rcx", "push rdx",
                "push rbp", "push rsi", "push rdi",
                "push r8", "push r9", "push r10", "push r11",
                "push r12", "push r13", "push r14", "push r15",
                "sub rsp, 256",
                "movdqu [rsp+0x00], xmm0",  "movdqu [rsp+0x10], xmm1",
                "movdqu [rsp+0x20], xmm2",  "movdqu [rsp+0x30], xmm3",
                "movdqu [rsp+0x40], xmm4",  "movdqu [rsp+0x50], xmm5",
                "movdqu [rsp+0x60], xmm6",  "movdqu [rsp+0x70], xmm7",
                "movdqu [rsp+0x80], xmm8",  "movdqu [rsp+0x90], xmm9",
                "movdqu [rsp+0xA0], xmm10", "movdqu [rsp+0xB0], xmm11",
                "movdqu [rsp+0xC0], xmm12", "movdqu [rsp+0xD0], xmm13",
                "movdqu [rsp+0xE0], xmm14", "movdqu [rsp+0xF0], xmm15",
                concat!("call ", $handler),
                "movdqu xmm0,  [rsp+0x00]", "movdqu xmm1,  [rsp+0x10]",
                "movdqu xmm2,  [rsp+0x20]", "movdqu xmm3,  [rsp+0x30]",
                "movdqu xmm4,  [rsp+0x40]", "movdqu xmm5,  [rsp+0x50]",
                "movdqu xmm6,  [rsp+0x60]", "movdqu xmm7,  [rsp+0x70]",
                "movdqu xmm8,  [rsp+0x80]", "movdqu xmm9,  [rsp+0x90]",
                "movdqu xmm10, [rsp+0xA0]", "movdqu xmm11, [rsp+0xB0]",
                "movdqu xmm12, [rsp+0xC0]", "movdqu xmm13, [rsp+0xD0]",
                "movdqu xmm14, [rsp+0xE0]", "movdqu xmm15, [rsp+0xF0]",
                "add rsp, 256",
                "pop r15", "pop r14", "pop r13", "pop r12",
                "pop r11", "pop r10", "pop r9", "pop r8",
                "pop rdi", "pop rsi", "pop rbp",
                "pop rdx", "pop rcx", "pop rbx", "pop rax",
                "iretq",
            );
        }
    };
}

isr_stub_light!(isr_timer, "timer_handler_inner");
isr_stub_full!(isr_keyboard, "keyboard_handler_inner");

/// Initialize GDT, IDT, PIC, PIT, and enable interrupts.
pub unsafe fn init() {
    unsafe {
        core::ptr::write_volatile(&raw mut KEYBUF_HEAD, 0);
        core::ptr::write_volatile(&raw mut KEYBUF_TAIL, 0);

        // Pre-compute addresses for ISR — avoids &raw mut codegen in interrupt handler
        let addrs = &raw mut ISR_ADDRS;
        (*addrs).keystate = (&raw mut KEYSTATE) as usize;
        (*addrs).shift_held = (&raw mut SHIFT_HELD) as usize;
        (*addrs).ctrl_held = (&raw mut CTRL_HELD) as usize;
        (*addrs).e0_prefix = (&raw mut E0_PREFIX) as usize;

        gdt_init();

        let idt = &raw mut IDT;
        for i in 0..256 {
            (*idt)[i] = EMPTY_IDT;
        }
        idt_set_gate(32, isr_timer as *const () as u64);
        idt_set_gate(33, isr_keyboard as *const () as u64);

        let idtp = &raw mut IDTP;
        (*idtp).limit = (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16;
        (*idtp).base = idt as u64;
        asm!("lidt [{}]", in(reg) idtp);

        pic_remap();
        pit_init();

        asm!("sti");
    }
}
