#![no_std]
#![no_main]

//! Link-only ESP32-P4 ECO5+ ROM USB ABI probe.
//!
//! This binary is NEVER loaded or executed on hardware. It exists solely to
//! force the linker to resolve the ROM CDC device pointer and FIFO fill call.

use core::{arch::global_asm, panic::PanicInfo};
use stub_core::tx::FifoFill;
use stub_rom::p4_usb::P4RomCdcTx;

global_asm!(
    r#"
    .section .text.entry, "ax", @progbits
    .global esp_main
    .type esp_main, @function
esp_main:
    la sp, __stack_top
    la t0, __bss_start
    la t1, __bss_end
1:
    bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b
2:
    call rust_entry
3:
    j 3b
"#
);

// Match the foundation's mandatory validation sentinels in the same linker
// script. The smoke executable does not replace the foundation executable.
#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".data.foundation_sentinel")]
pub static INITIALIZED_SENTINEL: u32 = 0x5354_5542;

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss.foundation_sentinel")]
pub static mut ZEROED_SENTINEL: [u32; 4] = [0; 4];

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() -> ! {
    // SAFETY: deliberately NOT safe to execute without an active USB ROM
    // session. This function must never be invoked on actual hardware.
    // The probe only checks that the ROM-owned pointer slot is loaded.
    if let Some(mut tx) = unsafe { P4RomCdcTx::from_active_rom_session() } {
        let bytes = core::hint::black_box(&[0xC0, 0x42, 0xC0]);
        let result = tx.fill(bytes);
        let _ = core::hint::black_box(result);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
