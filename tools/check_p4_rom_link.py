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


def check_elf(elf_path: Path, disassembly: str) -> None:
    with elf_path.open("rb") as binary:
        elf = ELFFile(binary)
        if elf["e_machine"] != "EM_RISCV":
            raise AssertionError("Smoke binary is not RISC-V")
        symbols = {}
        for section in elf.iter_sections():
            if section.header.sh_type == "SHT_SYMTAB":
                for sym in section.iter_symbols():
                    symbols[sym.name] = sym["st_value"]
    for name, expected in (
        ("uart_acm_dev", ROM_POINTER_SLOT),
        ("esp_rom_cdc_acm_fifo_fill", ROM_FIFO_FILL),
        ("esp_rom_cdc_acm_rx_fifo_cnt", ROM_RX_COUNT),
        ("esp_rom_cdc_acm_fifo_read", ROM_RX_READ),
    ):
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
    print("PASS: ROM pointer 0x4ffbffd4, TX 0x4fc008d0, RX count 0x4fc008d4, RX read 0x4fc008d8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("elf", type=Path)
    parser.add_argument("disassembly", type=Path)
    args = parser.parse_args()
    check_elf(args.elf, args.disassembly.read_text())


if __name__ == "__main__":
    main()
