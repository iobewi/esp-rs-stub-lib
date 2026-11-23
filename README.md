# esp-rs-stub-lib

**A Rust `no_std` foundation for RAM-resident flasher stubs on Espressif chips.**

The project aims to implement device-side flash stubs in Rust while keeping
compatibility with Espressif's existing host-side flashing workflow, including
`esptool-js` / Web Serial.

> **Development status:** the opt-in ESP32-P4 flasher candidate builds as a RAM image. Its integrated flash operations await ECO7 board qualification. [Commands, build and acceptance](docs/p4-flasher.md).

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

The integrated flasher links the portable packet codec, USB transport and P4 target operations. The foundation binary remains available for linker checks.

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

### Source attribution

The SLIP codec and RX double-buffer state machine in `stub-core` are a
**Rust port/adaptation** based on Espressif's C implementation:

- [`esp-flasher-stub/src/slip.c`](https://github.com/espressif/esp-flasher-stub/blob/master/src/slip.c)
- [`esp-flasher-stub/src/frame_buffer.c`](https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.c)
- [`esp-flasher-stub/src/frame_buffer.h`](https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.h)

The referenced C files carry Espressif copyright notices and are dual-licensed
under `Apache-2.0 OR MIT`. This repository uses the MIT option for the adapted
code. See [Espressif attribution and MIT permission notice](THIRD_PARTY_NOTICES.md).
The Rust implementation changes the ownership and error-handling API; it is
not an identical translation and has not been integrated with USB interrupts.

## License

Licensed under the [MIT License](LICENSE). Copyright © 2026 IOBEWI.
Espressif-originated material retains its respective copyright and permission
notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and source headers.
