#!/usr/bin/env python3
"""Static, never-run P4 ECO5+ ROM USB smoke-link check.

Do not interpret this as ECO7 runtime verification. ELF and disassembly must
contain the expected pointer slot and ROM call, not just linker-script text.
"""
from __future__ import annotations

import argparse
import re
from pathlib import Path

from elftools.elf.elffile import ELFFile

ROM_POINTER_SLOT = 0x4FFBFFD4
ROM_FIFO_FILL = 0x4FC008D0


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
    for name, expected in [
        ("uart_acm_dev", ROM_POINTER_SLOT),
        ("esp_rom_cdc_acm_fifo_fill", ROM_FIFO_FILL),
    ]:
        actual = symbols.get(name)
        if actual != expected:
            raise AssertionError(
                f"{name}: expected {expected:#x}, got {actual!r}"
            )
    # Symbol presence alone does not prove actual code uses them. Require a
    # ROM pointer *load* and a FIFO-call instruction in disassembly. The
    # textual checks are intentionally conservative and may need adjustment
    # if LLVM changes its disassembler annotation conventions.
    lines = disassembly.splitlines()
    pointer_refs = [
        i for i, line in enumerate(lines)
        if re.search(r"<uart_acm_dev(?:[+-]0x[0-9a-f]+)?>", line)
    ]
    fifo_refs = [
        i for i, line in enumerate(lines)
        if re.search(r"<esp_rom_cdc_acm_fifo_fill(?:[+-]0x[0-9a-f]+)?>", line)
    ]
    if not pointer_refs or not fifo_refs:
        print("DEBUG smoke ELF disassembly:\n" + disassembly)
        raise AssertionError("Both ROM symbols must appear in code disassembly")
    if not any(
        re.search(r"\blw\b", "\n".join(lines[max(0, i - 3): i + 4]))
        for i in pointer_refs
    ):
        raise AssertionError("Missing load of ROM CDC device pointer slot")
    if not any(
        re.search(r"\b(?:jal|jalr|call|c\.jalr)\b",
                  "\n".join(lines[max(0, i - 2): i + 3]))
        for i in fifo_refs
    ):
        raise AssertionError("Missing ROM FIFO-call instruction")
    print("PASS: P4 ECO5+ ROM pointer slot loaded and ROM FIFO function called")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("elf", type=Path)
    parser.add_argument("disassembly", type=Path)
    args = parser.parse_args()
    check_elf(args.elf, args.disassembly.read_text())


if __name__ == "__main__":
    main()
