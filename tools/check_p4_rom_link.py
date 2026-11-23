#!/usr/bin/env python3
"""Static check of *linked instructions* against ESP32-P4 ECO5+ ROM addresses.

Never executes the smoke ELF and never reads target memory. Extracts the
effective address of a LUI/LW pair and the PC-relative destination of an
AUIPC/JALR pair rather than relying on optional objdump symbol annotations.
"""
from __future__ import annotations

import argparse
import re
from pathlib import Path

from elftools.elf.elffile import ELFFile

ROM_POINTER_SLOT = 0x4FFBFFD4
ROM_FIFO_FILL = 0x4FC008D0
ROM_RX_COUNT = 0x4FC008D4
ROM_RX_READ = 0x4FC008D8

# P4 ECO5+ ROM table and ESP-IDF hw_ver3 register definitions, kept
# independent of the firmware's linker script and MMIO constants.
BRINGUP_CALLS = {
    "esp_rom_get_cpu_ticks_per_us": 0x4FC00040,
    "esp_rom_get_uart_device": 0x4FC000B4,
    "esp_rom_esprv_intc_int_set_priority": 0x4FC005AC,
    "esp_rom_isr_attach": 0x4FC005D0,
    "esp_rom_isr_mask": 0x4FC005D4,
    "esp_rom_isr_unmask": 0x4FC005D8,
    "esp_rom_cdc_acm_irq_state_enable": 0x4FC008E4,
    "esp_rom_cdc_acm_irq_rx_enable": 0x4FC008F0,
    "esp_rom_cdc_acm_irq_rx_disable": 0x4FC008F4,
    "esp_rom_cdc_acm_irq_callback_set": 0x4FC00900,
    "esp_rom_cdc_acm_line_ctrl_get": 0x4FC00908,
}
# Espressif esp-stub-lib 91e85dc3, ESP32-P4 ECO5 table + ECO7 extra symbol.
FLASHER_CALLS = {
    "spi_flash_attach": 0x4FC001E8,
    "spi_flash_boot_attach": 0x4FC0FE6E,
    "esp_rom_opiflash_exec_cmd_eco7": 0x4FC10B40,
    "esp_rom_spiflash_unlock": 0x4FC0015C,
    "esp_rom_spiflash_config_param": 0x4FC00168,
    "esp_rom_spiflash_read": 0x4FC00158,
    "esp_rom_spiflash_erase_chip": 0x4FC0012C,
    "GetSecurityInfoProc": 0x4FC000BC,
    "MD5Init": 0x4FC005E0,
    "MD5Update": 0x4FC005E4,
    "MD5Final": 0x4FC005E8,
    "tinfl_decompress": 0x4FC000F8,
}
ROM_DWC_HANDLER = 0x4FC00974
BRINGUP_MMIO = {
    0x5012D04C: {"lw"},        # EFUSE_RD_MAC_SYS_2_REG
    0x500E515C: {"lw", "sw"},  # HP_SYSTEM_SYS_USBOTG20_CTRL_REG
    0x500D6174: {"sw"},        # INTERRUPT_CORE0_USB_OTG_INT_MAP_REG
    0x50000008: {"lw", "sw"},  # USB GAHBCFG
}


INSTRUCTION = re.compile(
    r"^\s*([0-9a-fA-F]+):\s+([a-z][a-z0-9.]*)\s*(.*?)\s*$"
)
REG_IMM = re.compile(r"^\s*([a-z][a-z0-9]*),\s*(-?0x[0-9a-f]+)\s*$")
LOAD = re.compile(
    r"^\s*([a-z][a-z0-9]*),\s*(-?0x[0-9a-f]+)\(([a-z][a-z0-9]*)\)"
)


def instructions(disassembly: str) -> list[tuple[int, str, str]]:
    rows = []
    for line in disassembly.splitlines():
        match = INSTRUCTION.match(line)
        if match:
            rows.append((int(match[1], 16), match[2], match[3]))
    return rows


def upper_imm(value: str) -> int:
    # The U-type immediate is 20 bits and signed for AUIPC; for LUI,
    # taking it modulo 2**32 yields the same effective register value.
    raw = int(value, 0) & 0xFFFFF
    signed = raw - (1 << 20) if raw & (1 << 19) else raw
    return signed << 12


def find_pointer_load(disassembly: str, address: int = ROM_POINTER_SLOT) -> bool:
    ins = instructions(disassembly)
    for i, (_, op, args) in enumerate(ins):
        if op != "lui":
            continue
        upper = REG_IMM.fullmatch(args)
        if upper is None:
            continue
        register, immediate = upper.groups()
        # Only consecutive producer/consumer instructions are accepted.
        if i + 1 >= len(ins) or ins[i + 1][1] != "lw":
            continue
        load = LOAD.match(ins[i + 1][2])
        if load is None:
            continue
        _, offset, base = load.groups()
        calculated = (upper_imm(immediate) + int(offset, 0)) & 0xFFFFFFFF
        if base == register and calculated == address:
            return True
    return False


def find_rom_call(disassembly: str, address: int = ROM_FIFO_FILL) -> bool:
    ins = instructions(disassembly)
    for i, (pc, op, args) in enumerate(ins):
        if op == "jal":
            # An absolute destination may be printed by LLVM directly.
            destination = re.search(r"0x([0-9a-fA-F]+)", args)
            if destination and int(destination[1], 16) == address:
                return True
        if op != "auipc" or i + 1 >= len(ins):
            continue
        upper = REG_IMM.fullmatch(args)
        if upper is None:
            continue
        register, immediate = upper.groups()
        _, following_op, following_args = ins[i + 1]
        if following_op != "jalr":
            continue
        jump = re.search(
            r"(-?0x[0-9a-f]+)\(([a-z][a-z0-9]*)\)", following_args
        )
        if jump is None or jump[2] != register:
            continue
        calculated = (
            pc + upper_imm(immediate) + int(jump[1], 0)
        ) & 0xFFFFFFFE
        if calculated == address:
            return True
    return False


def find_mmio_accesses(disassembly: str, address: int) -> set[str]:
    """Recognize the candidate's straight-line LUI + word accesses."""
    rows = instructions(disassembly)
    accesses = set()
    for i, (pc, op, args) in enumerate(rows):
        upper = REG_IMM.fullmatch(args) if op == "lui" else None
        if upper is None:
            continue
        register, immediate = upper.groups()
        for next_pc, next_op, next_args in rows[i + 1:i + 9]:
            if next_pc - pc not in (2, 4):
                break
            pc = next_pc
            if next_op in ("lw", "sw"):
                access = LOAD.fullmatch(next_args)
                if access and access[3] == register:
                    effective = (upper_imm(immediate) + int(access[2], 0)) & 0xFFFFFFFF
                    if effective == address:
                        accesses.add(next_op)
            elif next_op not in ("lui", "li", "or", "ori", "addi", "mv"):
                break
            if next_op != "sw" and next_args.split(",", 1)[0].strip() == register:
                break
    return accesses


def find_handler_address(disassembly: str) -> bool:
    rows = instructions(disassembly)
    for (pc, op, args), (next_pc, next_op, next_args) in zip(rows, rows[1:]):
        upper = REG_IMM.fullmatch(args) if op == "lui" else None
        if upper and upper[1] == "a1" and next_pc == pc + 4 and next_op == "addi":
            parts = [part.strip() for part in next_args.split(",")]
            if len(parts) == 3 and parts[:2] == ["a1", "a1"]:
                if (upper_imm(upper[2]) + int(parts[2], 0)) & 0xFFFFFFFF == ROM_DWC_HANDLER:
                    return True
    return False


def check_elf(elf_path: Path, disassembly: str, bringup: bool = False, flasher: bool = False) -> None:
    with elf_path.open("rb") as binary:
        elf = ELFFile(binary)
        if elf["e_machine"] != "EM_RISCV":
            raise AssertionError("Smoke binary is not RISC-V")
        symbols = {}
        for section in elf.iter_sections():
            if section.header.sh_type == "SHT_SYMTAB":
                for sym in section.iter_symbols():
                    symbols[sym.name] = sym["st_value"]
    expected_symbols = (
        ("uart_acm_dev", ROM_POINTER_SLOT),
        ("esp_rom_cdc_acm_fifo_fill", ROM_FIFO_FILL),
        ("esp_rom_cdc_acm_rx_fifo_cnt", ROM_RX_COUNT),
        ("esp_rom_cdc_acm_fifo_read", ROM_RX_READ),
    )
    if bringup:
        expected_symbols += tuple(BRINGUP_CALLS.items())
        expected_symbols += (("esp_rom_usb_dw_isr_handler", ROM_DWC_HANDLER),)
    if flasher:
        expected_symbols += tuple(FLASHER_CALLS.items())
        for name, address in FLASHER_CALLS.items():
            if not find_rom_call(disassembly, address):
                raise AssertionError(f"Missing linked call to {name} at {address:#x}")
    for name, expected in expected_symbols:
        actual = symbols.get(name)
        if actual != expected:
            raise AssertionError(
                f"{name}: expected {expected:#x}, got {actual!r}"
            )
    if not find_pointer_load(disassembly):
        raise AssertionError("Missing LUI/LW from ROM pointer slot 0x4ffbffd4")
    if not find_rom_call(disassembly):
        raise AssertionError("Missing call targeting ROM FIFO at 0x4fc008d0")
    if not find_rom_call(disassembly, ROM_RX_COUNT):
        raise AssertionError("Missing call targeting ROM RX count at 0x4fc008d4")
    if not find_rom_call(disassembly, ROM_RX_READ):
        raise AssertionError("Missing call targeting ROM RX read at 0x4fc008d8")
    if bringup:
        for name, address in BRINGUP_CALLS.items():
            if not find_rom_call(disassembly, address):
                raise AssertionError(f"Missing linked call to {name} at {address:#x}")
        if not find_handler_address(disassembly):
            raise AssertionError("Missing DWC handler address materialization in a1")
        for address, expected in BRINGUP_MMIO.items():
            if not expected <= find_mmio_accesses(disassembly, address):
                raise AssertionError(f"Missing linked MMIO accesses at {address:#x}")
        print("PASS: P4 bring-up ROM calls, DWC handler pointer, and v3 MMIO accesses")
    print("PASS: ROM pointer 0x4ffbffd4, TX 0x4fc008d0, RX count 0x4fc008d4, RX read 0x4fc008d8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("elf", type=Path)
    parser.add_argument("disassembly", type=Path)
    parser.add_argument("--bringup", action="store_true")
    parser.add_argument("--flasher", action="store_true")
    args = parser.parse_args()
    check_elf(args.elf, args.disassembly.read_text(), args.bringup, args.flasher)


if __name__ == "__main__":
    main()
