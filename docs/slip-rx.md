# SLIP and RX double buffering (software-only PR)

The `stub-core` crate provides:

- `slip::SlipDecoder`: deterministic byte-oriented SLIP decoder; nonempty
  frames only, malformed escapes rejected, no implicit start delimiter. After
  `C0 C0`, the state returns to `NoFrame`; subsequent data is ignored
  until the next `C0`.
- `slip::send_frame`: encoder with fallible `TxSink` and explicit flush.
  Unlike the C byte-transmit callback, write errors are propagated.
- `rx::DoubleBuffer<N>`: two fixed-size inline slots, `poll`,
  `frame`, and idempotent `release_selected`. The selection is stable
  across consecutive `poll()` calls only if no intervening RX/release
  changes the slot states. Newly received errors have priority; release
  always acts on the latest poll selection.

The reference is Espressif `esp-flasher-stub/src/slip.c`,
`frame_buffer.c`, and `transport.c`. Frame capacity is 16,647 bytes.

**Concurrency boundary:** this API uses exclusive `&mut self` and is
**not ISR-safe**. It does not claim lock-free access, atomics, DMA-safety,
or provide a USB interface. The P4 platform adapter must define
interrupt masking/critical sections or an equivalent ownership protocol
before calling this from a real interrupt context.

No heap allocation is required on target; large `DoubleBuffer`
instances should be kept in static RAM, not on an interrupt stack.

The first P4 foundation image is deliberately unchanged and still does
not send OHAI, access USB, or read/write flash.

Test using `cargo test -p stub-core` and
`cargo test --workspace --exclude p4-foundation`.
