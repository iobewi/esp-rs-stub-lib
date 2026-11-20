# ESP32-P4 USB ROM adapter — PR #7 (software-only)

## Purpose

The purpose of this Rust stub is to fix the **USB memory/flash-read
communication problems** observed on P4, not simply port Espressif's C
stub. This PR establishes an opt-in ROM TX adapter and ABI boundary.
It does **not** activate USB at boot, install RX interrupts, send OHAI,
read flash, or measure hardware throughput.

## Source-to-symbol audit

The Espressif [P4 ECO5+ linker table](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/ld/esp32p4.rom.eco5.ld)
defines:

| Symbol | ECO5+ address | Meaning |
| --- | --- | --- |
| `cdc_acm_fifo_fill` | `0x4FC008D0` | `int (cdc_acm_device *, const uint8_t *, int)` |
| `cdc_acm_rx_fifo_cnt` | `0x4FC008D4` | ROM RX FIFO count |
| `cdc_acm_fifo_read` | `0x4FC008D8` | ROM RX FIFO read |
| `uart_acm_dev` | `0x4FFBFFD4` | ROM **pointer slot** to CDC ACM device |

The [API alias table](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/ld/esp32p4.rom.api.ld)
maps `esp_rom_cdc_acm_fifo_fill` to `cdc_acm_fifo_fill`.
The [P4 target wrapper](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/src/usb_otg.c)
sets up interrupt routing, ROM callbacks and DWC USB masks separately.
The [shared USB C source](https://github.com/espressif/esp-stub-lib/blob/master/src/usb_otg.c)
declares the FIFO fill signature and contains the unchecked TX flush.

**Critical revision distinction:** the older `esp32p4.rom.ld` table
uses different addresses, including `cdc_acm_fifo_fill = 0x4FC008DC`
and `uart_acm_dev = 0x4FF3FFD4`. The new Rust declarations MUST NOT
be reused indiscriminately across revisions. ECO7 support is inferred
from Espressif's ECO5+ symbol table, not independently confirmed on
hardware.

## Implemented contract

- `stub-rom::p4_usb::P4RomCdcTx` implements the portable
  `stub-core::tx::FifoFill` interface.
- Construction is `unsafe`, explicit, and restricted by documented
  preconditions to an already-active compatible P4 ROM USB session.
- The adapter reads the *value stored in* `uart_acm_dev`, not the
  device pointer slot address itself. Null returns `None`.
- One call of `fill` forwards at most one ROM invocation. A negative
  ROM return is **always classified indeterminate**, never retryable
  without external evidence of zero bytes consumed.
- `TxQueue` handles partial/zero acceptance, caps offered length to
  64 bytes by default and locks after indeterminate outcomes.
- The existing P4 RAM binary gains a Cargo dependency on `stub-rom`,
  but **does not call the adapter** or establish a USB session. There
  is no claim of a usable esptool-js stub yet.

## Deliberately deferred boundaries

- ROM boot channel detection (`GetUartDevice`/buffer identity), revision
  detection and USB session validity
- USB DWC interrupt mapping, ROM callback installation, RTS reset handling,
  RX byte pumping into SLIP double buffers
- Safe ownership/interrupt-exclusion policy for `TxQueue` and RX slots;
  the current modules are exclusive/sequential, not ISR-safe
- Frame/session reset after ambiguous partial transmission
- Host handshake and actual read-only memory/flash command handling

## Performance qualification plan

The previously observed ~45.4 KiB/s, ~16,640 calls and ~1.24 ms/call are
**historical measurements**, not freshly reproduced by this PR. The
first controlled experiment must compare 64- and 4,096-byte application
staging buffers while holding the **ROM maximum offer at 64 bytes**;
only then increase the offer limit after the real ROM's acceptance
behavior has been characterized. Record offered/accepted byte totals,
per-call latency (including distribution, not just mean), backpressure,
RX activity, final payload hash and MiB/s under matched conditions.

Any IDLab hardware recipe must be read-only and require explicit target
and session authorization. No flash erase/write or unsupervised firmware
deployment is authorized by this PR.

## Independent ABI link verification

The separate `p4-rom-link-smoke` test executable is **opt-in**, guarded by
`required-features = ["rom-link-smoke"]`. Normal `cargo p4-build` builds
only `p4-foundation`; the CI checks that no smoke executable exists after
that default build. Only the dedicated CI step uses
`cargo build --release --target riscv32imc-unknown-none-elf -p p4-foundation --features rom-link-smoke --bin p4-rom-link-smoke`.

The test executable is built for RV32IMC using
the same `p4-memory.x` and `p4-rom.x` as the real foundation. It contains
an intentionally unsafe and **never executed** path that constructs
`P4RomCdcTx` and calls `FifoFill::fill`. This forces both the volatile
ROM data-pointer read and the actual ROM FIFO function call into the linked
ELF. The CI checks their exact symbol addresses and verifies the load/call
instructions in the disassembly. The ELF and disassembly are archived.

This smoke ELF must **never** be distributed as the firmware or passed to
IDLab for loading; its entry point presumes a valid ROM USB session.

The original `p4-foundation` executable remains non-operational and does
not call the ROM USB functions. This check validates only link-time ABI
resolution, not ECO7 execution, ROM FIFO semantics or hardware performance.
