#![no_std]
#![no_main]

//! Minimal non-operational RAM image for validating P4 linker/loader layout.
//! It deliberately does not install USB interrupts, send OHAI, or modify flash.

use core::arch::global_asm;
use core::panic::PanicInfo;

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

// Dedicated input sections are retained explicitly by KEEP in p4-memory.x.
// The ELF-to-JSON converter validates the actual linked symbols and bytes.
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
