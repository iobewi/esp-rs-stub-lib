//! Single-owner USB RX coordination across an interrupt notification.
//!
//! The interrupt side touches only atomics. A platform callback MUST mask
//! the CDC RX interrupt before calling `notify_rx_masked`. It must not call
//! RxPump, DoubleBuffer or a ROM FIFO read.
//!
//! Foreground owns all mutable RX state and operates with CDC RX masked.
//! A platform implementation must guarantee that `mask_rx` excludes an
//! in-flight callback before foreground accesses the ROM device.
//!
//! Not an ISR installer: P4 DWC/CLIC routing, callback registration, USB
//! session checks and RTS-reset policy remain outside this portable module.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::rx::DoubleBuffer;
use crate::usb_rx::{RxPump, RxPumpError, RxPumpResult, RxSource};

/// ISR-to-foreground notification; the producer never borrows the frame
/// buffers. A platform callback must mask the *RX source* before notifying.
pub struct RxIrqSignal {
    pending: AtomicBool,
    link_changed: AtomicBool,
}

impl Default for RxIrqSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl RxIrqSignal {
    pub const fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            link_changed: AtomicBool::new(false),
        }
    }

    /// Called by the ISR only *after* masking the CDC RX source.
    pub fn notify_rx_masked(&self) {
        self.pending.store(true, Ordering::Release);
    }

    /// Called from the ROM callback for a link-state event; do not
    /// interpret this as a reset until line status is checked by foreground.
    pub fn notify_line_state_changed(&self) {
        self.link_changed.store(true, Ordering::Release);
    }

    pub fn link_change_pending(&self) -> bool {
        self.link_changed.load(Ordering::Acquire)
    }

    /// Only the foreground owner may clear this flag, and only AFTER
    /// synchronously masking/quiescing the RX interrupt. RV32IMC has no
    /// atomic read-modify-write instruction; a blind load/store while
    /// an ISR can publish would otherwise lose notifications.
    fn take_pending_masked(&self) -> bool {
        let pending = self.pending.load(Ordering::Acquire);
        self.pending.store(false, Ordering::Release);
        pending
    }

    /// The caller has synchronously quiesced *all* CDC callbacks including
    /// line-state callbacks, not merely RX. Only foreground may clear this.
    fn acknowledge_line_change_masked(&self) {
        self.link_changed.store(false, Ordering::Release);
    }

    /// An explicit externally synchronized reset may clear these flags.
    /// Exclusive access prevents safe Rust producers from signalling during
    /// reset; hardware integration must additionally mask and quiesce ISR.
    pub fn reset_after_quiesce(&mut self) {
        *self.pending.get_mut() = false;
        *self.link_changed.get_mut() = false;
    }
}

/// Hardware adapter. `mask_rx` must synchronously exclude an in-flight
/// callback before foreground ROM accesses; `unmask_rx` must preserve
/// pending level-triggered RX status. Neither property is proven for ECO7.
pub trait RxIrqMask {
    fn mask_rx(&mut self);
    fn unmask_rx(&mut self);
}

/// The outcome of checking the line *after* all relevant CDC callbacks
/// have been synchronously quiesced, never before.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineDecision {
    /// Current line state and session are valid; preserve received data.
    Benign,
    /// RTS reset edge, detach, invalid session, or uncertain state.
    ResyncRequired,
}

/// Hardware-specific critical section for CDC line-state inspection.
/// Unlike RxIrqMask::mask_rx, this must exclude ALL CDC callback producers,
/// including line changes. The exclusion remains active through the
/// subsequent line-state read and atomic acknowledgement.
pub trait RxLineState: RxIrqMask {
    fn quiesce_cdc_callbacks(&mut self);
    fn inspect_line_state_quiesced(&mut self) -> LineDecision;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceResult {
    /// No IRQ/pending data; hardware may remain unmasked.
    Idle,
    /// No new FIFO call: decoder cannot accept any more bytes yet.
    Paused { pending: usize },
    /// One bounded ROM source read, or a drain of cached batch data.
    Progress { received: usize },
    /// A ROM read reported no progress despite positive FIFO occupancy.
    /// RX remains masked and the foreground must try again or time out.
    NoProgress,
    /// FIFO confirmed empty, ROM RX may now be re-enabled.
    Drained,
    /// Link-state event requires foreground inspection; not necessarily reset.
    LinkChange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    Rx(RxPumpError),
    ResyncRequired,
}

/// Foreground-only state. NEVER share between ISR and main.
/// Use one owner for RxPump, DoubleBuffer and the ROM FIFO adapter.
pub struct RxIrqCoordinator {
    draining: bool,
    fault: bool,
    masked: bool,
}

impl Default for RxIrqCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl RxIrqCoordinator {
    /// Begin with RX *masked* by the platform before the first call.
    pub const fn new() -> Self {
        Self {
            draining: false,
            fault: false,
            masked: true,
        }
    }

    pub const fn is_masked(&self) -> bool {
        self.masked
    }

    pub const fn requires_resync(&self) -> bool {
        self.fault
    }

    fn mask<M: RxIrqMask>(&mut self, mask: &mut M) {
        // Always ask the platform to synchronize an in-flight callback
        // before touching the FIFO, even if our cached state says masked.
        mask.mask_rx();
        self.masked = true;
    }

    /// Resolve a line-state notification within one quiescence interval.
    ///
    /// First mask RX, then synchronously quiesce ALL CDC callbacks; only
    /// afterwards inspect the line and session. If benign, clear ONLY the
    /// line flag while callbacks are still quiesced, retaining RX events,
    /// SLIP frames, pump bytes and drain state. On reset/uncertainty, keep
    /// the event latched; external resynchronization is required.
    ///
    /// The platform MUST keep line callbacks excluded from return of
    /// quiesce_cdc_callbacks() until this method has cleared the flag.
    /// Hardware validation of this contract on ECO7 is still outstanding.
    pub fn resolve_line_change<M: RxLineState>(
        &mut self,
        signal: &RxIrqSignal,
        mask: &mut M,
    ) -> LineDecision {
        self.mask(mask);
        mask.quiesce_cdc_callbacks();
        let decision = mask.inspect_line_state_quiesced();
        if decision == LineDecision::Benign {
            signal.acknowledge_line_change_masked();
        }
        decision
    }

    /// One bounded foreground step. It never calls a FIFO reader if RX is
    /// disarmed, and never unmasks RX with cached bytes still pending.
    pub fn service_once<S: RxSource, M: RxIrqMask, const N: usize>(
        &mut self,
        signal: &RxIrqSignal,
        mask: &mut M,
        source: &mut S,
        pump: &mut RxPump,
        frames: &mut DoubleBuffer<N>,
    ) -> Result<ServiceResult, ServiceError> {
        if self.fault || pump.requires_resync() {
            self.mask(mask);
            return Err(ServiceError::ResyncRequired);
        }
        if signal.link_change_pending() {
            self.mask(mask);
            return Ok(ServiceResult::LinkChange);
        }

        if !frames.is_armed() {
            self.mask(mask);
            return Ok(ServiceResult::Paused {
                pending: pump.pending(),
            });
        }

        // No notification is needed to drain a previously accepted ROM
        // batch after DoubleBuffer::poll() has rearmed a spare frame slot.
        // A pending notification is acknowledged only under a synchronous
        // mask: RV32IMC cannot use AtomicBool::swap.
        if !self.draining && pump.pending() == 0 && !signal.pending.load(Ordering::Acquire) {
            // Bootstrap: startup requires RX masked until the callback and
            // session are installed, but the first foreground iteration
            // must arm an idle, empty receiver to receive the first IRQ.
            if self.masked {
                mask.unmask_rx();
                self.masked = false;
                if signal.pending.load(Ordering::Acquire) || signal.link_change_pending() {
                    self.mask(mask);
                }
            }
            return Ok(ServiceResult::Idle);
        }

        self.mask(mask);
        if signal.take_pending_masked() {
            self.draining = true;
        }
        // A line event may have arrived before the synchronous mask
        // completed; never read from a session with pending line changes.
        if signal.link_change_pending() {
            return Ok(ServiceResult::LinkChange);
        }
        let outcome = match pump.pump_once(source, frames) {
            Ok(result) => result,
            Err(error) => {
                self.fault = true;
                return Err(ServiceError::Rx(error));
            }
        };

        match outcome {
            RxPumpResult::Paused { pending } => Ok(ServiceResult::Paused { pending }),
            RxPumpResult::NoProgress => Ok(ServiceResult::NoProgress),
            RxPumpResult::Progress { received } => {
                // Even if a complete frame disarmed RX, keep masked until a
                // spare slot is rearmed and all buffered data is consumed.
                Ok(ServiceResult::Progress { received })
            }
            RxPumpResult::Empty => {
                self.draining = false;
                if !frames.is_armed() || pump.pending() != 0 {
                    return Ok(ServiceResult::Paused {
                        pending: pump.pending(),
                    });
                }
                // Enable only after a verified empty read while armed.
                mask.unmask_rx();
                self.masked = false;
                // A callback racing with unmask can mask RX and publish
                // an event. Never clear an event after enabling; if it
                // arrived already, mask again and drain next iteration.
                if signal.pending.load(Ordering::Acquire) || signal.link_change_pending() {
                    self.mask(mask);
                }
                Ok(ServiceResult::Drained)
            }
        }
    }

    /// External link/framing resynchronization must have occurred and the
    /// hardware RX interrupt must be quiesced before calling this method.
    /// The exclusive signal borrow prevents concurrent Rust notification.
    pub fn reset_after_resync<M: RxIrqMask, const N: usize>(
        &mut self,
        signal: &mut RxIrqSignal,
        mask: &mut M,
        pump: &mut RxPump,
        frames: &mut DoubleBuffer<N>,
    ) {
        self.mask(mask);
        signal.reset_after_quiesce();
        pump.reset_after_resync();
        frames.reset_after_resync();
        self.draining = false;
        self.fault = false;
    }
}
