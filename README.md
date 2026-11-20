# esp-rs-stub-lib

**A Rust `no_std` foundation for RAM-resident flasher stubs on Espressif chips.**

The project aims to implement device-side flash stubs in Rust while keeping
compatibility with Espressif's existing host-side flashing workflow, including
`esptool-js` / Web Serial.

> **Development status:** early foundation. This repository does **not** yet
> contain a functional P4 USB flasher. Do not deploy the foundation image on
> a production device or use it as a replacement for Espressif's stub.

## Architecture

| Path | Purpose |
| --- | --- |
| [`crates/stub-core/`](crates/stub-core/) | Portable SLIP codec, bounded RX double buffer, and shared protocol primitives |
| [`crates/stub-rom/`](crates/stub-rom/) | ROM declarations; signatures and addresses require per-chip verification before use |
| [`targets/esp32p4/`](targets/esp32p4/) | First-class ESP32-P4 RV32IMC RAM firmware target (`p4-foundation`) |
| [`tools/`](tools/) | ELF validation, conversion to esptool-js stub JSON, and regression comparison |
| [`tests/`](tests/) | Python validation tests |
| [`docs/`](docs/) | Foundation and SLIP/RX design notes |

Future targets may be added beside `targets/esp32p4/`, without duplicating
portable protocol logic.

## Current implementation

- Rust `no_std` workspace, with a minimal ESP32-P4 RAM entry trampoline,
  stack initialization and BSS clearing.
- Linker layout and an ELF-to-JSON converter that checks the entry point,
  memory sections and retained sentinels.
- Portable SLIP encoding/decoding and bounded double-buffered receive logic
  in `stub-core`, tested independently of the P4 firmware.
- Software CI for host tests, RV32IMC cross-compilation, formatting, and
  ELF/JSON validation.

**Not implemented yet:** a functional USB transport, command dispatcher,
flash operations, and interrupt-safe integration of RX buffers. The portable
`stub-core` is not linked into the P4 foundation binary yet. ROM USB ABI
details and actual hardware behavior remain to be validated.

## Development

Requirements: Rust with the `riscv32imc-unknown-none-elf` target, Python 3,
and `pyelftools` for inspecting the ELF.

```sh
cargo fmt --all -- --check
cargo test --workspace --exclude p4-foundation
cargo check --release --target riscv32imc-unknown-none-elf -p stub-core
cargo build --release --target riscv32imc-unknown-none-elf -p p4-foundation

python -m pip install pyelftools
python -m unittest discover -s tests -v
mkdir -p build
python tools/elf2stub.py \
  target/riscv32imc-unknown-none-elf/release/p4-foundation \
  build/p4-foundation.json
```

The JSON is a **build/validation artifact only** at this stage: the firmware
does not complete the host handshake or implement flashing commands.

More details: [P4 foundation](docs/foundation.md) and
[SLIP and RX double buffering](docs/slip-rx.md).

## References

- [Espressif esp-flasher-stub](https://github.com/espressif/esp-flasher-stub)
- [Espressif esp-stub-lib](https://github.com/espressif/esp-stub-lib)
- [Espressif esptool-js](https://github.com/espressif/esptool-js)

This is an independent Rust implementation effort; refer to the upstream
projects for their respective source licenses and hardware documentation.

## License

Licensed under the [MIT License](LICENSE). Copyright © 2026 IOBEWI.
