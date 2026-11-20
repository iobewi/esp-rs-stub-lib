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

The codec and double-buffer logic are Rust adaptations of Espressif's
[`slip.c`](https://github.com/espressif/esp-flasher-stub/blob/master/src/slip.c),
[`frame_buffer.c`](https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.c)
and [`frame_buffer.h`](https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.h).
The relevant C files contain Espressif copyright notices and an
`Apache-2.0 OR MIT` license identifier; this project selects MIT and retains
the upstream copyright and permission notices in
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).
Frame capacity is 16,647 bytes.

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
