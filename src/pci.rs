// pci.rs - PCI configuration space access and device enumeration
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

use crate::io::{outl, inl};

const PCI_CONFIG_ADDR: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

#[derive(Copy, Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
}

fn pci_addr(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
}

pub fn pci_read(dev: &PciDevice, offset: u8) -> u32 {
    unsafe {
        outl(PCI_CONFIG_ADDR, pci_addr(dev.bus, dev.slot, dev.func, offset));
        inl(PCI_CONFIG_DATA)
    }
}

pub fn pci_write(dev: &PciDevice, offset: u8, val: u32) {
    unsafe {
        outl(PCI_CONFIG_ADDR, pci_addr(dev.bus, dev.slot, dev.func, offset));
        outl(PCI_CONFIG_DATA, val);
    }
}

pub fn pci_read_bar(dev: &PciDevice, bar: u8) -> u32 {
    pci_read(dev, 0x10 + bar * 4)
}

pub fn pci_enable_device(dev: &PciDevice) {
    let cmd = pci_read(dev, 0x04);
    // Set I/O space (bit 0), memory space (bit 1), bus master (bit 2)
    pci_write(dev, 0x04, cmd | 0x07);
}

/// Call `f` for every present PCI device (multi-function aware).
pub fn pci_enumerate(mut f: impl FnMut(&PciDevice)) {
    for bus in 0..=255u16 {
        for slot in 0..32u8 {
            let max_func = max_functions(bus as u8, slot);
            for func in 0..max_func {
                let dev = PciDevice { bus: bus as u8, slot, func };
                let id = pci_read(&dev, 0x00);
                if id == 0xFFFFFFFF || (id & 0xFFFF) == 0xFFFF {
                    continue;
                }
                f(&dev);
            }
        }
    }
}

/// Scan PCI bus for a device matching class and subclass.
pub fn pci_find_device(class: u8, subclass: u8) -> Option<PciDevice> {
    let mut result = None;
    pci_enumerate(|dev| {
        if result.is_some() { return; }
        let class_reg = pci_read(dev, 0x08);
        let dev_class = ((class_reg >> 24) & 0xFF) as u8;
        let dev_subclass = ((class_reg >> 16) & 0xFF) as u8;
        if dev_class == class && dev_subclass == subclass {
            result = Some(*dev);
        }
    });
    result
}

/// Scan PCI bus for a device matching vendor and device ID.
pub fn pci_find_vendor(vendor: u16, device: u16) -> Option<PciDevice> {
    let mut result = None;
    pci_enumerate(|dev| {
        if result.is_some() { return; }
        let id = pci_read(dev, 0x00);
        let dev_vendor = (id & 0xFFFF) as u16;
        let dev_device = ((id >> 16) & 0xFFFF) as u16;
        if dev_vendor == vendor && dev_device == device {
            result = Some(*dev);
        }
    });
    result
}

fn max_functions(bus: u8, slot: u8) -> u8 {
    let dev = PciDevice { bus, slot, func: 0 };
    let vendor = pci_read(&dev, 0x00);
    if vendor == 0xFFFFFFFF || (vendor & 0xFFFF) == 0xFFFF {
        return 0;
    }
    let header = pci_read(&dev, 0x0C);
    if (header >> 16) & 0x80 != 0 {
        8 // multi-function device
    } else {
        1
    }
}
