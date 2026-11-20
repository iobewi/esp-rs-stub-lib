#![no_std]

//! Declaration-only ROM boundary. Never assume ROM pointers are safe.
//! The ESP32-P4 symbol addresses are supplied by p4-rom.x.
//! No ROM functions are invoked by the PR1 foundation binary.

use core::ffi::c_void;

pub type CdcAcmDevice = c_void;

unsafe extern "C" {
    pub fn esp_rom_cdc_acm_fifo_fill(
        dev: *mut CdcAcmDevice,
        tx_data: *const u8,
        length: i32,
    ) -> i32;

    pub fn esp_rom_cdc_acm_fifo_read(
        dev: *mut CdcAcmDevice,
        rx_data: *mut u8,
        length: i32,
    ) -> i32;

    pub fn esp_rom_cdc_acm_rx_fifo_cnt(dev: *mut CdcAcmDevice) -> i32;
}
