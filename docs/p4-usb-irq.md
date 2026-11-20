# PR #10 — USB P4 ROM interrupt/foreground ownership

## Source comparison

Espressif `esp-stub-lib/src/usb_otg.c` performs ROM FIFO reads in
the CDC ACM callback on status `ACM_STATUS_RX = -4`, passing bytes
directly into SLIP. Its P4 wrapper installs the ROM DWC handler,
CLIC routing, USB global mask, CDC callback and RX enable. The
P4-specific registers, callback reentrancy, IRQ mask semantics, RTS
handling and ECO7 behavior are **not** validated on our hardware.

## Coordination protocol

`RxIrqSignal` contains only atomic event flags. The ISR must first
**mask the CDC RX source**, and only then call `notify_rx_masked()`.
It must not borrow `DoubleBuffer`, `RxPump`, or call ROM FIFO reads.
A link-state event is latched for foreground investigation, not
automatically interpreted as a USB reset.

`RxIrqCoordinator` is foreground-only. It owns no ROM pointer but
coordinates an exclusive borrowed `RxPump`, `DoubleBuffer`, and
`RxSource`. It performs a synchronous hardware mask before each ROM
read and forbids unmasking while SLIP is disarmed or buffered data remains.
It services each wake through one bounded read per foreground step, then
re-enables RX **only after a verified empty read with an armed buffer**.
Notifications arriving during unmask are retained for the next step.

In case of an indeterminate RX result, no retry or re-enable is allowed.
Recovery requires external transport resynchronization and hardware
quiescence, then an explicit coordinator reset. Clearing atomic flags
without IRQ quiescence would lose an event.

## Strict safety boundary

This PR is deliberately limited to the **portable IRQ handoff contract**.
There is NO ROM callback installation, no DWC/CLIC register programming,
no ISR or ROM execution in `p4-foundation`, and no hardware access.
The `RxIrqMask` adapter is a required platform capability; its
synchronous exclusion of in-flight callbacks, USB RX status delivery,
and level-triggered re-pend behavior **must be verified on ECO7**
before connecting it to real hardware. A plain interrupt-disable call
is not automatically proof of such synchronization.

The current test matrix covers multiple SLIP frames across one ROM
read, no FIFO calls while disarmed, strict rearm-after-empty, faults,
reset events, and coalesced notifications under Miri.

## Future gated P4 implementation

1. Verify ROM active USB session and ECO7 addresses.
2. Prove CLIC interrupt masking and in-flight callback quiescence.
3. Add a platform callback that masks CDC RX before posting an atomic
   notification; handle other CDC statuses and RTS changes separately.
4. Gate enable/disable to the single ROM CDC device owner, with
   conservative error and disconnection recovery.
5. Add read-only IDLab measurements and verify physical FIFO wakeups
   under backpressure, packet coalescing and rapid reconnect.
6. Only after successful qualification activate the stub transport.

Startup requires a fully configured but masked ROM callback. The first
foreground `service_once()` unmasks RX when the receiver is empty,
armed and has no pending link change; otherwise the first IRQ could
never occur. A notification racing with unmask is retained, and an
observed link-change event prevents ROM FIFO access.

RV32IMC has no atomic read-modify-write operation for the event flag.
The foreground acknowledges RX notifications by atomic load/store **only
after** the platform synchronously masks and quiesces the CDC RX
callback. Attempting to clear a pending flag while the ISR can write it
would lose wakeups. The portable code is not proof that the platform's
mask operation has those exclusion semantics.
