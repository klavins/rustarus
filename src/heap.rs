// heap.rs - First-fit free-list heap allocator (matching icarus malloc.c)
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

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

const ALIGN: usize = 8;
const BLOCK_MAGIC: u32 = 0xA110CA7E;

#[repr(C)]
struct Block {
    size: usize,
    free: u32,
    magic: u32,
    next: *mut Block,
}

const HEADER_SIZE: usize = (core::mem::size_of::<Block>() + (ALIGN - 1)) & !(ALIGN - 1);

fn align_up(val: usize) -> usize {
    (val + (ALIGN - 1)) & !(ALIGN - 1)
}

static mut FREE_LIST: *mut Block = core::ptr::null_mut();

unsafe fn split(b: *mut Block, size: usize) {
    unsafe {
        if (*b).size < size + HEADER_SIZE + ALIGN {
            return;
        }
        let remaining = (*b).size - size - HEADER_SIZE;
        let new = (b as *mut u8).add(HEADER_SIZE + size) as *mut Block;
        (*new).size = remaining;
        (*new).free = 1;
        (*new).magic = BLOCK_MAGIC;
        (*new).next = (*b).next;
        (*b).size = size;
        (*b).next = new;
    }
}

unsafe fn coalesce(b: *mut Block) {
    unsafe {
        // Forward: merge with consecutive free blocks
        while !(*b).next.is_null() && (*(*b).next).free == 1 {
            (*b).size += HEADER_SIZE + (*(*b).next).size;
            (*b).next = (*(*b).next).next;
        }
        // Backward: find predecessor and merge if free
        let list = ptr::read_volatile(&raw const FREE_LIST);
        let mut prev = list;
        while !prev.is_null() && (*prev).next != b {
            prev = (*prev).next;
        }
        if !prev.is_null() && (*prev).free == 1 {
            (*prev).size += HEADER_SIZE + (*b).size;
            (*prev).next = (*b).next;
        }
    }
}

pub unsafe fn heap_init(base: *mut u8, size: usize) {
    unsafe {
        let addr = align_up(base as usize);
        let adjusted = size - (addr - base as usize);
        let block = addr as *mut Block;
        (*block).size = adjusted - HEADER_SIZE;
        (*block).free = 1;
        (*block).magic = BLOCK_MAGIC;
        (*block).next = ptr::null_mut();
        ptr::write_volatile(&raw mut FREE_LIST, block);
    }
}

pub unsafe fn heap_alloc(size: usize) -> *mut u8 {
    unsafe {
        if size == 0 {
            return ptr::null_mut();
        }
        let aligned = align_up(size);
        let list = ptr::read_volatile(&raw const FREE_LIST);
        let mut b = list;
        while !b.is_null() {
            if (*b).free == 1 && (*b).size >= aligned {
                split(b, aligned);
                (*b).free = 0;
                return (b as *mut u8).add(HEADER_SIZE);
            }
            b = (*b).next;
        }
        ptr::null_mut()
    }
}

pub unsafe fn heap_free(p: *mut u8) {
    unsafe {
        if p.is_null() {
            return;
        }
        let b = p.sub(HEADER_SIZE) as *mut Block;
        if (*b).magic != BLOCK_MAGIC {
            return;
        }
        (*b).free = 1;
        coalesce(b);
    }
}

pub unsafe fn heap_realloc(p: *mut u8, size: usize) -> *mut u8 {
    unsafe {
        if p.is_null() {
            return heap_alloc(size);
        }
        if size == 0 {
            heap_free(p);
            return ptr::null_mut();
        }
        let b = p.sub(HEADER_SIZE) as *mut Block;
        if (*b).magic != BLOCK_MAGIC {
            return ptr::null_mut();
        }
        let aligned = align_up(size);
        if (*b).size >= aligned {
            split(b, aligned);
            return p;
        }
        // Try to grow in place
        if !(*b).next.is_null() && (*(*b).next).free == 1
            && (*b).size + HEADER_SIZE + (*(*b).next).size >= aligned
        {
            (*b).size += HEADER_SIZE + (*(*b).next).size;
            (*b).next = (*(*b).next).next;
            split(b, aligned);
            return p;
        }
        let new = heap_alloc(size);
        if new.is_null() {
            return ptr::null_mut();
        }
        ptr::copy_nonoverlapping(p, new, (*b).size);
        heap_free(p);
        new
    }
}

pub fn heap_free_total() -> usize {
    let mut total = 0;
    unsafe {
        let list = ptr::read_volatile(&raw const FREE_LIST);
        let mut b = list;
        while !b.is_null() {
            if (*b).free == 1 {
                total += (*b).size;
            }
            b = (*b).next;
        }
    }
    total
}

pub fn heap_used_total() -> usize {
    let mut total = 0;
    unsafe {
        let list = ptr::read_volatile(&raw const FREE_LIST);
        let mut b = list;
        while !b.is_null() {
            if (*b).free == 0 {
                total += (*b).size;
            }
            b = (*b).next;
        }
    }
    total
}

pub struct HeapAllocator;

unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { heap_alloc(layout.size()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { heap_free(ptr) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, _layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { heap_realloc(ptr, new_size) }
    }
}
