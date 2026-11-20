# PR1 scope: P4 RAM foundation

This is a software-only foundation. **It is not a flash stub.** In particular,
it intentionally does not answer `OHAI`, handle SLIP, flash memory, or USB
interrupts. Never substitute this image into a production IDLab session.

## Source compatibility reference

- Espressif `esp-flasher-stub/src/ld/esp32p4.ld` and `common.ld`.
- Espressif `esp-stub-lib/src/target/esp32p4/ld/esp32p4.rom.eco5.ld`.
- `esptool-js/src/stubFlasher.ts` and `esploader.ts::runStub()`.

ESP32-P4 revision 3.x uses 0x4FF50000/0x4500 IRAM, and
0x4FF60000/0x18000 DRAM in the C reference.
This prototype reserves the final 0x1000 DRAM bytes for its own stack.
The section loader sends `.text` and `.data`, never `.bss`.
The assembly entry initializes SP and zeroes BSS before branching into Rust.
The linker retains dedicated data/BSS sentinels with KEEP, and the converter
verifies the final ELF symbol addresses, lengths, and initialized bytes.

## Build (Rust toolchain required)

```sh
cargo test --workspace --exclude p4-foundation
cargo fmt --all -- --check
cargo build --release --target riscv32imc-unknown-none-elf -p p4-foundation
python -m pip install pyelftools
python -m unittest discover -s tests -v
mkdir -p build
python tools/elf2stub.py target/riscv32imc-unknown-none-elf/release/p4-foundation build/p4-foundation.json
```

The JSON shape is compatible with the basic esptool-js stub resource format,
but **this non-operational image must not be loaded onto real hardware**.
The workspace target is not a replacement for the C ROM flasher.

## Open checks before hardware execution

- Verify LLVM/Rust cross-toolchain output, RV32IMC instructions and ILP32 ABI.
- Confirm ELF entry trampoline and stack disassembly, section sizes, and generated JSON.
- Determine the exact ECO7 ROM function signatures and interrupt ABI before calling them.
- Add SLIP and bounded TX handling in a subsequent change.
- Validate external-stub selection in a guarded, non-production esptool-js integration.
- Never perform destructive programming without explicit per-board authorization.
