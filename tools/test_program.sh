#!/usr/bin/env bash
# Pass the checkout of Espressif esp-stub-lib used for the ROM miniz reference.
set -euo pipefail
upstream=${1:?esp-stub-lib checkout required}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cc -O2 -ffunction-sections -fdata-sections -I"$upstream/include/esp-stub-lib" -c "$upstream/src/target/esp8266/src/miniz.c" -o "$tmp/miniz.o"
ar crs "$tmp/libtinfl.a" "$tmp/miniz.o"
rustc --edition=2021 --crate-name stub_core --crate-type rlib crates/stub-core/src/lib.rs -o "$tmp/libstub_core.rlib"
rustc --edition=2021 tests/native/program.rs --extern "stub_core=$tmp/libstub_core.rlib" -L "$tmp" -l static=tinfl -o "$tmp/test-program"
python - "$tmp" <<'PY'
import pathlib, random, sys, zlib
root = pathlib.Path(sys.argv[1])
payload = random.Random(17).randbytes(40000) + b"USB" * 16001
(root / "plain").write_bytes(payload)
(root / "compressed").write_bytes(zlib.compress(payload))
PY
"$tmp/test-program" "$tmp/plain" "$tmp/compressed"
if [[ -n "${2:-}" ]]; then
    rustc --edition=2021 tests/native/wire.rs --extern "stub_core=$tmp/libstub_core.rlib" -L "$tmp" -l static=tinfl -o "$tmp/wire"
    node tests/native/esptool.mjs "$2" "$tmp/wire" "$tmp/plain"
fi
