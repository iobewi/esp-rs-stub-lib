# PR #9 — bounded USB ROM RX bridge (software-only)

Goal: remove USB communication loss/corruption while preparing a trustworthy
P4 ECO7 memory/flash-read throughput investigation. This PR is deliberately
**not** a complete USB ISR integration or an esptool-compatible RAM stub.

## Upstream source audit

Espressif `esp-stub-lib/src/usb_otg.c` handles CDC RX status `-4` in its
callback; it loops on `esp_rom_cdc_acm_rx_fifo_cnt(dev)`, reads one byte
with `esp_rom_cdc_acm_fifo_read(dev, &c, 1)`, then feeds the SLIP decoder.
The P4-specific `src/target/esp32p4/src/usb_otg.c` installs interrupt
routing and callback registration separately.

We **do not reuse its ISR synchronization assumption**. The portable Rust
`DoubleBuffer` currently supports only sequential ownership and cannot be
shared between an ISR and the foreground without a proven critical-section or
lock-free handoff mechanism.

## This change

- `stub-core::usb_rx::RxPump` calls an injected `RxSource` **once** per
  foreground iteration, using a 64-byte stack batch. The limit is software
  policy, not a claimed ECO7 ROM capacity.
- Bytes are passed to the existing SLIP `DoubleBuffer` in arrival order,
  across partial reads and escaped-byte boundaries.
- Negative/ambiguous counts poison the pump. The pump refuses all further
  reads until *external transport resynchronization* and explicit reset.
- `stub-rom::p4_usb::P4RomCdcTx` implements `RxSource` with one
  `esp_rom_cdc_acm_rx_fifo_cnt` and at most one
  `esp_rom_cdc_acm_fifo_read` per operation. Negative ROM returns are
  **never considered automatically retryable**; a read count exceeding the
  offered slice is rejected.
- RX and TX share a single `&mut P4RomCdcTx` to ensure sequential access
  at the Rust API boundary; this is not interrupt-safe by itself.
- Tests cover bounded reads, split SLIP frames, split escape sequences,
  zero-read, a read error after implicit FIFO consumption, and over-reporting.
  The portable tests run in the nightly Miri CI job.

## Explicitly not included

No hardware/USB initialization, IRQ activation, USB session detection,
RTS reset, DMA, flash erase/write/read commands, OHAI handshake, or
additional executable firmware artifact. `p4-foundation` remains
non-operational. Future IRQ integration must first prove FIFO ownership
exclusion and priority/critical-section behavior on P4 ECO7.

## Hardware qualification gate

When a transport owner and safe ROM activation exist, perform read-only
IDLab qualification with explicit device/session consent. Measure source
count/read call frequency and latency, FIFO acceptance, complete SLIP
payload hashes and end-to-end throughput. Preserve the historical TX
reference (~45.4 KiB/s, ~1.24 ms/call), with a controlled 64-byte
ROM-offer baseline before increasing any offer limits.
