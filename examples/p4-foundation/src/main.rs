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
    call rust_entry
1:
    j 1b
"#
);

// Explicitly retain small initialization sentinels to exercise both JSON
// .data upload and local .bss clearing in the foundation linker contract.
#[used]
#[unsafe(link_section = ".data")]
static INITIALIZED_SENTINEL: u32 = 0x5354_5542;

#[used]
#[unsafe(link_section = ".bss")]
static mut ZEROED_SENTINEL: [u32; 4] = [0; 4];

unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() -> ! {
    // SAFETY: Symbols are defined by p4-memory.x, word-aligned, and describe
    // only the dedicated NOLOAD .bss section in writable DRAM.
    unsafe {
        let mut cursor = core::ptr::addr_of!(__bss_start) as *mut u32;
        let end = core::ptr::addr_of!(__bss_end) as usize;
        while (cursor as usize) < end {
            core::ptr::write_volatile(cursor, 0);
            cursor = cursor.add(1);
        }
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
