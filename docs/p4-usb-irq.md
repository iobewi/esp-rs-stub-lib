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

## Review corrections: zero-progress reads and benign line changes

`RxSource::read` now reports `RxRead::Empty` only when FIFO occupancy
was measured as zero. A read returning zero bytes after positive occupancy
reports `RxRead::NoProgress`. The coordinator retains its draining state,
keeps interrupts masked, and retries via subsequent foreground iterations
without relying on another interrupt. The caller must bound prolonged
no-progress periods with a policy above this layer; no automatic reset or
infinite blocking loop is introduced.

A benign CDC line-state notification is resolved via
`RxIrqCoordinator::resolve_line_change`, which masks RX, synchronously
quiesces **all** CDC callbacks (including line-state callbacks), and only
then inspects RTS/session state under that exclusion. Only a benign
inspection can clear the line-state flag, before the exclusion ends.
A reset edge, detach, or ambiguous state leaves the flag pending.
The method preserves buffered
SLIP frames, RX pending notifications, the pump remainder and its draining
state. A real USB reset, detach or invalid session still requires external
transport recovery followed by `reset_after_resync` with its deliberate
FIFO and software-buffer purge.

## P2 race correction: inspection and acknowledgement under one exclusion

`RxLineState` separates RX masking from full CDC callback quiescence.
`resolve_line_change` invokes `mask_rx`, then
`quiesce_cdc_callbacks`, then `inspect_line_state_quiesced`, and clears
the notification only on `LineDecision::Benign` while the callback
producers remain quiescent. An RTS reset edge arriving during quiescence
is therefore included in the inspection, not silently acknowledged on a
stale benign state. An invalid/uncertain session remains latched for reset.
This is a required platform contract, not evidence that the ECO7 ROM
already supports such exclusion.

## P2 recovery latch: a stable line cannot undo a detected reset

A `LineDecision::ResyncRequired` sets `RxIrqCoordinator::fault` under
CDC callback quiescence. Subsequent calls to `resolve_line_change` cannot
clear the line notification or reinterpret an earlier reset as benign,
even if RTS has returned to a stable level. `service_once` remains blocked
and keeps RX masked. Only `reset_after_resync`, after **external** session
re-establishment and stale FIFO disposal, clears this latch together with
the software RX buffers. A regression test covers reset detected, physical
line subsequently stable, repeated inspection rejected, and explicit reset.
