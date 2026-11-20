#!/usr/bin/env python3
"""Convert the *strict* ESP32-P4 foundation ELF layout into esptool-js stub JSON.

This command does not upload or execute code. Requires pyelftools.
"""
import argparse
import base64
import json
from pathlib import Path

IRAM = (0x4FF50000, 0x4FF54500)
DRAM = (0x4FF60000, 0x4FF77000)
STACK = (0x4FF77000, 0x4FF78000)


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
    if not (text_start <= entry < text_start + len(text)):
        raise ValueError("entry is not in text")
    if text_start % 4 or data_start % 4 or bss_start % 4:
        raise ValueError("load addresses must be word-aligned")


def convert(path):
    from elftools.elf.elffile import ELFFile

    with open(path, "rb") as stream:
        elf = ELFFile(stream)
        if elf.elfclass != 32 or elf.header["e_machine"] != "EM_RISCV":
            raise ValueError("expected ELF32 RISC-V")
        sections = {section.name: section for section in elf.iter_sections()}
        for name in (".text", ".bss", ".data"):
            if name not in sections:
                raise ValueError(f"missing {name} section")
        # The JS loader uploads only .text and .data. Reject any other
        # SHF_ALLOC PROGBITS/NOLOAD section so no loadable data is dropped.
        for section in elf.iter_sections():
            if section.name in (".text", ".bss", ".data"):
                continue
            if section["sh_flags"] & 0x2 and section["sh_size"]:
                raise ValueError(f"unsupported allocated section: {section.name}")

        text = sections[".text"]
        data = sections[".data"]
        bss = sections[".bss"]
        if bss["sh_type"] != "SHT_NOBITS":
            raise ValueError("BSS must be SHT_NOBITS")
        payload = bytes(text.data())
        initialized = bytes(data.data())
        text_start = text["sh_addr"]
        data_start = data["sh_addr"]
        bss_start = bss["sh_addr"]
        check_layout(text_start, payload, bss_start, bss["sh_size"],
                     data_start, initialized, elf.header["e_entry"])
        return {
            "entry": elf.header["e_entry"],
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
