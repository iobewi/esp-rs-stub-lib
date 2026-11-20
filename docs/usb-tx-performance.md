# P4 USB-OTG TX: integrity before throughput

## Why this exists

The goal of the Rust stub is to solve the ESP32-P4 USB memory/flash-read
communication problem, not just reproduce Espressif's C implementation.

Baseline measurements from the existing qualification work (not rerun in
this PR): about **45.4 KiB/s**, **23.094 s/MiB**, **16,640 FIFO fill calls**
and **1.24 ms per call**. Changing the read portions to 1,024 or 4,096
bytes did not improve these observations. The approximate fill-call timing
and call count are empirical, not proven ROM internals.

Relevant upstream implementation:
- [esp-stub-lib/src/usb_otg.c](https://github.com/espressif/esp-stub-lib/blob/master/src/usb_otg.c)
- [esp-stub-lib/src/target/esp32p4/src/usb_otg.c](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/src/usb_otg.c)
- [esp-flasher-stub/src/transport.c](https://github.com/espressif/esp-flasher-stub/blob/master/src/transport.c)
- [P4 ECO5+ ROM linker symbols](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/ld/esp32p4.rom.eco5.ld)

### Proven defect in upstream C

`stub_lib_usb_otg_tx_flush()` sends a 64-byte staging buffer using
`esp_rom_cdc_acm_fifo_fill(dev, data, length)`, **discards the function's
integer result**, then resets the buffered length to zero unconditionally.
If the ROM accepts fewer bytes, unsent bytes are lost. This is an integrity
risk independent of any performance investigation.

### What PR #6 provides

`stub-core::tx::TxQueue<N>` implements a sequential, allocation-free
queue with a `FifoFill` injection point. It retains any unaccepted bytes,
treats zero acceptance as backpressure, and reports negative/invalid
returns without silently clearing pending data. `pump_once()` makes at
most one FIFO call; the caller controls retry policy so zero acceptance
does not cause a busy-spin. A retryable error must carry an adapter-backed
**zero-byte consumption guarantee**. Indeterminate negative errors and
over-acceptance poison the queue: no replay, no enqueue, no further FIFO
call until the transport has re-established a new framing/session and
explicitly invoked `reset_after_resync()`. That reset discards all
ambiguous queued bytes; it cannot resynchronize USB or SLIP by itself.
It exposes:
- `fill_calls`, `accepted_bytes`
- `short_accepts`, `zero_accepts`, `invalid_accepts`

Unit tests model 64-, 32-, 1- and 0-byte acceptance, invalid return
values, saturation/backpressure and lossless ordered recovery. Additional
fault-injection tests simulate bytes being consumed before an invalid
return, and prove that poisoned queues never replay them.

The *experimental* staging capacity is 4,096 bytes, but **the default
per-call ROM offer limit is 64 bytes**, configurable independently by
`TxQueue::<N>::with_max_offer(limit)`. The first 64-vs-4,096 comparison
therefore uses the same 64-byte maximum ROM offer in both cases.
Only after ECO7 acceptance sizes are established should the per-call
limit be increased. The ROM is not proven to accept 4,096 bytes in one call.

### Integration gates (not performed in PR #6)

1. Verify P4 ECO7 ROM call ABI and the data pointer `uart_acm_dev`.
   The C signature is `int fifo_fill(cdc_acm_device *, const uint8_t *, int)`
   and the P4 ECO5+ linker lists the symbol. This does not by itself prove
   ECO7 runtime behavior or any acceptance upper bound.
2. Add a thin chip-specific ROM adapter, with explicit buffer ownership
   and interrupt exclusions. Never hold shared TX bytes in an ISR without
   proven synchronization.
3. Integrate `stub-core` into the actual P4 RAM executable and validate
   ELF/.text/.data/BSS budgets and esptool-js handshake.
4. Instrument each actual FIFO call: attempted length, accepted length,
   call count, duration distribution, stall/error counts and end-to-end
   throughput. Compare **64 vs 4,096 application-side staging** while
   holding the ROM offer limit at **64 bytes** and keeping payload and
   operating conditions identical. Only then sweep the ROM offer limit.
5. Run read-only qualification under explicit IDLab device/session
   authorization, preserving baseline hashes. No flash erase/write.

**Important:** Fixing partial accepts addresses loss/corruption.
It does **not** explain or automatically eliminate the observed 1.24 ms
per ROM call. A throughput gain requires measured reduction in call
count, latency or both.

This PR is software-only: no ROM call, no USB ISR, no flash access.
