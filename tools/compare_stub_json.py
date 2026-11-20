#!/usr/bin/env python3
"""Require byte-for-byte identical loader-visible P4 JSON after relocation."""
import argparse
import base64
import hashlib
import json
from pathlib import Path

ADDRESS_FIELDS = ("entry", "text_start", "data_start", "bss_start")
PAYLOAD_FIELDS = ("text", "data")
EXPECTED_FIELDS = set(ADDRESS_FIELDS + PAYLOAD_FIELDS)


def compare(before, after):
    if set(before) != EXPECTED_FIELDS or set(after) != EXPECTED_FIELDS:
        raise ValueError("unexpected JSON fields")
    report = {}
    for field in ADDRESS_FIELDS:
        if type(before[field]) is not int or type(after[field]) is not int:
            raise ValueError(f"{field}: expected integer")
        if before[field] != after[field]:
            raise ValueError(f"{field}: address differs")
        report[field] = f"0x{before[field]:08X}"
    for field in PAYLOAD_FIELDS:
        try:
            old = base64.b64decode(before[field], validate=True)
            new = base64.b64decode(after[field], validate=True)
        except (TypeError, ValueError) as exc:
            raise ValueError(f"{field}: invalid payload") from exc
        if old != new:
            raise ValueError(f"{field}: binary payload differs")
        report[field] = {"length": len(old), "sha256": hashlib.sha256(old).hexdigest()}
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    result = compare(json.loads(args.before.read_text()), json.loads(args.after.read_text()))
    output = json.dumps(result, indent=2, sort_keys=True) + "\n"
    print(output)
    if args.report:
        args.report.write_text(output)


if __name__ == "__main__":
    main()
