#!/usr/bin/env python3
"""Convert the strict ESP32-P4 foundation ELF layout into esptool-js stub JSON.

This command does not upload or execute code. Requires pyelftools.
"""
import argparse
import base64
import json
import struct
from pathlib import Path

IRAM = (0x4FF50000, 0x4FF54500)
DRAM = (0x4FF60000, 0x4FF77000)
STACK = (0x4FF77000, 0x4FF78000)
DATA_SENTINEL = struct.pack("<I", 0x53545542)
BSS_SENTINEL_SIZE = 16


def within(start, size, region):
    return size >= 0 and region[0] <= start <= region[1] and start + size <= region[1]


def check_layout(text_start, text, bss_start, bss_size, data_start, data, entry):
    if not text or not within(text_start, len(text), IRAM):
        raise ValueError("text outside IRAM")
    if not within(bss_start, bss_size, DRAM):
        raise ValueError("BSS outside available DRAM")
    if not within(data_start, len(data), DRAM):
        raise ValueError("data outside available DRAM")
    if bss_start + bss_size > data_start:
        raise ValueError("BSS overlaps data")
    if entry % 2 or not (text_start <= entry < text_start + len(text)):
        raise ValueError("entry is not an aligned address in text")
    if text_start % 4 or data_start % 4 or bss_start % 4 or bss_size % 4:
        raise ValueError("load addresses and BSS size must be word-aligned")


def check_elf_header(elf):
    if elf.elfclass != 32 or elf.header["e_machine"] != "EM_RISCV":
        raise ValueError("expected ELF32 RISC-V")
    if not elf.little_endian:
        raise ValueError("expected little-endian ELF")
    if elf.header["e_type"] != "ET_EXEC":
        raise ValueError("expected executable ELF (ET_EXEC)")


def check_entry(entry, symbol_address, text_start, text_length):
    if entry % 2 or entry != symbol_address:
        raise ValueError("entry must be the aligned esp_main symbol")
    if not text_start <= entry < text_start + text_length:
        raise ValueError("esp_main must be within text")


def check_sentinel(name, symbol, section, expected_size, expected_bytes=None):
    if symbol is None:
        raise ValueError(f"missing {name} symbol")
    if symbol["st_size"] != expected_size:
        raise ValueError(f"{name} has unexpected size")
    addr = symbol["st_value"]
    start = section["sh_addr"]
    if not start <= addr or addr + expected_size > start + section["sh_size"]:
        raise ValueError(f"{name} lies outside its expected section")
    if expected_bytes is not None:
        offset = addr - start
        if bytes(section.data())[offset:offset + expected_size] != expected_bytes:
            raise ValueError(f"{name} initialized bytes differ from expected data")


def convert(path):
    from elftools.elf.elffile import ELFFile

    with open(path, "rb") as stream:
        elf = ELFFile(stream)
        check_elf_header(elf)
        sections = {section.name: section for section in elf.iter_sections()}
        for name in (".text", ".bss", ".data"):
            if name not in sections:
                raise ValueError(f"missing {name} section")

        # JS uploads only .text and .data; no other allocated section can
        # be silently omitted. Keep .bss non-loadable.
        for section in elf.iter_sections():
            if section.name in (".text", ".bss", ".data"):
                continue
            if section["sh_flags"] & 0x2 and section["sh_size"]:
                raise ValueError(f"unsupported allocated section: {section.name}")

        text, data, bss = (sections[name] for name in (".text", ".data", ".bss"))
        if text["sh_type"] != "SHT_PROGBITS" or data["sh_type"] != "SHT_PROGBITS":
            raise ValueError("text and data must be SHT_PROGBITS")
        if bss["sh_type"] != "SHT_NOBITS":
            raise ValueError("BSS must be SHT_NOBITS")

        symbols = elf.get_section_by_name(".symtab")
        if symbols is None:
            raise ValueError("symbol table is required for foundation verification")

        def symbol(name):
            matches = symbols.get_symbol_by_name(name)
            if not matches or len(matches) != 1:
                raise ValueError(f"missing or ambiguous {name} symbol")
            return matches[0]

        entry = elf.header["e_entry"]
        check_entry(entry, symbol("esp_main")["st_value"],
                    text["sh_addr"], text["sh_size"])
        check_sentinel("INITIALIZED_SENTINEL", symbol("INITIALIZED_SENTINEL"),
                       data, len(DATA_SENTINEL), DATA_SENTINEL)
        check_sentinel("ZEROED_SENTINEL", symbol("ZEROED_SENTINEL"),
                       bss, BSS_SENTINEL_SIZE)

        payload = bytes(text.data())
        initialized = bytes(data.data())
        text_start = text["sh_addr"]
        data_start = data["sh_addr"]
        bss_start = bss["sh_addr"]
        check_layout(text_start, payload, bss_start, bss["sh_size"],
                     data_start, initialized, entry)

        if not payload or not initialized or bss["sh_size"] < BSS_SENTINEL_SIZE:
            raise ValueError("foundation sentinel sections are empty")
        return {
            "entry": entry,
            "text_start": text_start,
            "text": base64.b64encode(payload).decode("ascii"),
            "data_start": data_start,
            "data": base64.b64encode(initialized).decode("ascii"),
            "bss_start": bss_start,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    obj = convert(args.elf)
    args.output.write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
