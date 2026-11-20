// SPDX-FileCopyrightText: 2026 IOBEWI
// SPDX-License-Identifier: MIT

//! ESP32-P4 ECO5+ ROM USB-OTG CDC ACM TX boundary.
//!
//! The symbols are provided by `targets/esp32p4/p4-rom.x`. This adapter is
//! opt-in: the foundation firmware does not invoke it or install a USB ISR.
//! **ECO7 is covered only by the upstream ECO5+ compatibility claim; actual
//! ROM ABI, session ownership, and behavior require hardware validation.**

use core::{ptr, ptr::NonNull};

use stub_core::tx::{FifoFailure, FifoFill};

use crate::{esp_rom_cdc_acm_fifo_fill, CdcAcmDevice};

unsafe extern "C" {
    /// A ROM *data symbol* holding a device pointer, not a device instance.
    /// Linker maps the symbol address to 0x4FFBFFD4 for P4 ECO5+.
    static mut uart_acm_dev: *mut CdcAcmDevice;
}

/// A raw negative result does not guarantee that zero bytes were consumed.
/// Treat it as indeterminate until proven otherwise by a ROM contract.
pub fn classify_rom_result(result: i32) -> Result<usize, FifoFailure> {
    if result < 0 {
        Err(FifoFailure::Indeterminate(result))
    } else {
        Ok(result as usize)
    }
}

/// An exclusively owned wrapper for the ROM CDC ACM device.
///
/// The lifetime cannot be enforced by Rust: the ROM owns the allocation and
/// may reset, detach, or mutate it via interrupt handlers. The caller must
/// uphold those invariants and coordinate access before constructing this.
pub struct P4RomCdcTx {
    device: NonNull<CdcAcmDevice>,
}

impl P4RomCdcTx {
    /// # Safety
    ///
    /// Only for a verified ESP32-P4 ECO5+ ROM download session, with USB-OTG
    /// active and a valid stable `uart_acm_dev`. The caller must guarantee
    /// that the ROM device and TX FIFO can be accessed for the entire lifetime
    /// of this adapter, including necessary exclusion from interrupts/reset.
    /// This function does not validate chip revision or install an ISR.
    pub unsafe fn from_active_rom_session() -> Option<Self> {
        // Read the *contents* of the ROM-owned pointer slot. Using its address
        // directly as a CDC device would be incorrect.
        let device = unsafe { ptr::read_volatile(ptr::addr_of!(uart_acm_dev)) };
        NonNull::new(device).map(|device| Self { device })
    }
}

impl FifoFill for P4RomCdcTx {
    /// Submits exactly one offered slice to the ROM function; no retry here.
    fn fill(&mut self, bytes: &[u8]) -> Result<usize, FifoFailure> {
        // The queue caps the offer at i32::MAX. Defend against direct trait
        // callers violating that limit, without making any ROM call.
        let len = match i32::try_from(bytes.len()) {
            Ok(length) => length,
            Err(_) => return Err(FifoFailure::Indeterminate(-1)),
        };
        // SAFETY: construction requires a stable, valid ROM session and a
        // single owner; this method preserves the borrow for the call only.
        let result =
            unsafe { esp_rom_cdc_acm_fifo_fill(self.device.as_ptr(), bytes.as_ptr(), len) };
        classify_rom_result(result)
    }
}

impl stub_core::usb_rx::RxSource for P4RomCdcTx {
    /// Poll the ROM USB CDC RX FIFO without installing an ISR.
    ///
    /// Returns at most the caller's buffer length, with one FIFO read at
    /// most. A nonpositive count performs no FIFO read (negative is poison).
    /// This shares the same ROM session safety preconditions as TX.
    fn read(&mut self, bytes: &mut [u8]) -> Result<stub_core::usb_rx::RxRead, stub_core::usb_rx::RxFailure> {
        use stub_core::usb_rx::{RxFailure, RxRead};
        if bytes.is_empty() {
            return Ok(RxRead::NoProgress);
        }
        // SAFETY: the P4RomCdcTx constructor's session/exclusivity
        // preconditions also cover RX FIFO calls on this ROM device.
        let available = unsafe { crate::esp_rom_cdc_acm_rx_fifo_cnt(self.device.as_ptr()) };
        if available < 0 {
            return Err(RxFailure::Indeterminate(available));
        }
        if available == 0 {
            return Ok(RxRead::Empty);
        }
        let offered = core::cmp::min(bytes.len(), available as usize);
        let offered = core::cmp::min(offered, i32::MAX as usize);
        // SAFETY: exclusive, stable ROM device; caller gives a writable
        // slice valid for the duration of this one ROM FIFO read.
        let result = unsafe {
            crate::esp_rom_cdc_acm_fifo_read(
                self.device.as_ptr(),
                bytes.as_mut_ptr(),
                offered as i32,
            )
        };
        if result < 0 {
            return Err(RxFailure::Indeterminate(result));
        }
        let received = result as usize;
        if received > offered {
            return Err(RxFailure::InvalidCount {
                offered,
                returned: received,
            });
        }
        if received == 0 {
            // RX count was positive; never mistake a non-progressing FIFO
            // read for confirmed emptiness.
            return Ok(RxRead::NoProgress);
        }
        Ok(RxRead::Data(received))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rom_acceptance_is_unsigned_when_nonnegative() {
        assert_eq!(classify_rom_result(0), Ok(0));
        assert_eq!(classify_rom_result(64), Ok(64));
        assert_eq!(classify_rom_result(i32::MAX), Ok(i32::MAX as usize));
    }

    #[test]
    fn negative_rom_return_is_never_assumed_retryable() {
        assert_eq!(classify_rom_result(-5), Err(FifoFailure::Indeterminate(-5)));
    }
}
