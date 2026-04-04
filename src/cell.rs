// cell.rs - Single-core static cell wrapper
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

use core::cell::UnsafeCell;

/// Single-core wrapper for global mutable state.
/// No locking — assumes single CPU, no SMP.
pub struct StaticCell<T>(UnsafeCell<T>);

// Safety: single-core, no SMP. Only one execution context at a time.
unsafe impl<T> Sync for StaticCell<T> {}

impl<T> StaticCell<T> {
    pub const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    /// Get a mutable reference to the inner value.
    /// Safety: caller must ensure no concurrent access.
    pub unsafe fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}
