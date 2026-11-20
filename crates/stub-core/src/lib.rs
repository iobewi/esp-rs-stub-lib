#![no_std]

//! Portable, pure definitions for a RAM-resident flasher stub.
//! Hardware access and protocol implementation belong in later PRs.

pub mod rx;
pub mod slip;
pub mod tx;

pub const SLIP_END: u8 = 0xC0;
pub const SLIP_ESC: u8 = 0xDB;
pub const SLIP_ESC_END: u8 = 0xDC;
pub const SLIP_ESC_ESC: u8 = 0xDD;
pub const HANDSHAKE: [u8; 4] = *b"OHAI";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressRange {
    pub start: u32,
    pub size: u32,
}

impl AddressRange {
    pub const fn end(self) -> Option<u32> {
        self.start.checked_add(self.size)
    }

    pub const fn contains(self, address: u32) -> bool {
        match self.end() {
            Some(end) => address >= self.start && address < end,
            None => false,
        }
    }

    pub const fn overlaps(self, other: Self) -> bool {
        if self.size == 0 || other.size == 0 {
            return false;
        }
        match (self.end(), other.end()) {
            (Some(end), Some(other_end)) => self.start < other_end && other.start < end,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_boundaries() {
        let r = AddressRange {
            start: 0x1000,
            size: 0x100,
        };
        assert!(r.contains(0x1000));
        assert!(!r.contains(0x1100));
        assert!(!r.overlaps(AddressRange {
            start: 0x1100,
            size: 8,
        }));
        assert!(r.overlaps(AddressRange {
            start: 0x10ff,
            size: 2,
        }));
    }

    #[test]
    fn rejects_address_overflow() {
        let r = AddressRange {
            start: u32::MAX,
            size: 2,
        };
        assert_eq!(r.end(), None);
        assert!(!r.contains(u32::MAX));
    }

    #[test]
    fn empty_ranges_never_overlap() {
        let r = AddressRange {
            start: 0x1000,
            size: 0x100,
        };
        assert!(!r.overlaps(AddressRange {
            start: 0x1080,
            size: 0,
        }));
        assert!(!AddressRange {
            start: 0x1080,
            size: 0,
        }
        .overlaps(r));
        assert!(!AddressRange { start: 0, size: 0 }.overlaps(AddressRange { start: 0, size: 0 }));
    }
}
