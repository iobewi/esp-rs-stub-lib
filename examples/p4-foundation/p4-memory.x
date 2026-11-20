/* ESP32-P4 revision >= 3.0; ROM-download RAM image (Espressif reference).
 * Reserve 4 KiB for the stack outside the linker's allocatable DRAM.
 * Stub image is deliberately non-operational at this stage.
 */
OUTPUT_ARCH(riscv)
ENTRY(esp_main)

MEMORY
{
  IRAM (rx)  : ORIGIN = 0x4FF50000, LENGTH = 0x4500
  DRAM (rw) : ORIGIN = 0x4FF60000, LENGTH = 0x17000
}

__stack_bottom = ORIGIN(DRAM) + LENGTH(DRAM);
__stack_top = __stack_bottom + 0x1000;

SECTIONS
{
  .text : ALIGN(4)
  {
    KEEP(*(.text.entry))
    *(.text .text.*)
    *(.rodata .rodata.*)
    . = ALIGN(4);
  } > IRAM

  .bss (NOLOAD) : ALIGN(4)
  {
    __bss_start = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > DRAM

  .data : ALIGN(4)
  {
    *(.sdata .sdata.*)
    *(.data .data.*)
    . = ALIGN(4);
  } > DRAM

  /DISCARD/ :
  {
    *(.eh_frame .eh_frame.*)
    *(.comment)
    *(.riscv.attributes)
  }
}

ASSERT(__bss_end <= ORIGIN(DRAM) + LENGTH(DRAM), "BSS exceeds P4 DRAM");
ASSERT(SIZEOF(.data) + ADDR(.data) <= __stack_bottom, "Data overlaps reserved stack");
ASSERT(ADDR(.text) + SIZEOF(.text) <= ORIGIN(IRAM) + LENGTH(IRAM), "Text exceeds P4 IRAM");
