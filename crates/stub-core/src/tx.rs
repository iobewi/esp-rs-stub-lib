//! Lossless, nonblocking staging for a FIFO that reports accepted byte counts.
//!
//! A call to `pump_once` makes *at most one* ROM/FIFO call. If only part of the
//! buffer is accepted, the remaining bytes are retained, in order. This
//! prevents the data loss caused by discarding a buffer after partial fill.
//! No USB hardware access or ISR synchronization occurs in this module.

/// An initial experimental staging size, not a claim about ROM FIFO capacity.
/// The platform should benchmark it against 64-byte staging on real hardware.
/// The default maximum offered per ROM call remains 64 bytes.
pub const USB_TX_STAGING_BYTES: usize = 4096;

/// Models the signed `int` result of Espressif's CDC ACM ROM FIFO fill.
///
/// The return value is the number of bytes accepted, potentially zero. A
/// negative return or a result larger than the offered slice is an error.
/// Implementations must not retain the slice after returning.
/// `NoProgress` must only be returned with a *proven* zero-consumption
/// guarantee. A raw ROM negative code is `Indeterminate` by default.
pub trait FifoFill {
    fn fill(&mut self, bytes: &[u8]) -> Result<usize, FifoFailure>;
}

/// The adapter must prove the zero-consumption guarantee to use NoProgress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FifoFailure {
    NoProgress(i32),
    Indeterminate(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxConfigError {
    InvalidMaxOffer,
}

/// An indeterminate ROM outcome prevents replay until the *external* link
/// has been resynchronized. This is not a recoverable FIFO backpressure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxState {
    Ready,
    ResyncRequired,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TxCounters {
    pub fill_calls: u64,
    pub accepted_bytes: u64,
    pub short_accepts: u64,
    pub zero_accepts: u64,
    pub invalid_accepts: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PumpResult {
    /// No pending data; no underlying FIFO call was made.
    Empty,
    /// The FIFO accepted zero bytes. Caller decides when to retry.
    Backpressure { remaining: usize },
    /// Accepted positive bytes. Remaining data must still be pumped.
    Progress { accepted: usize, remaining: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxError {
    /// Proven zero bytes consumed: data stays queued and may be retried.
    NoProgress(i32),
    /// Unknown number of bytes may have been consumed; replay forbidden.
    Indeterminate(i32),
    /// The adapter claimed to accept more bytes than were offered.
    InvalidAcceptance { offered: usize, returned: usize },
    /// Transfer already in terminal error state; no new FIFO calls allowed.
    ResyncRequired,
}

/// Sequential TX owner. NOT safe to share between interrupt and foreground
/// code without a platform-specific critical section / ownership protocol.
pub struct TxQueue<const N: usize> {
    bytes: [u8; N],
    head: usize,
    tail: usize,
    counters: TxCounters,
    max_offer: usize,
    state: TxState,
}

impl<const N: usize> Default for TxQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> TxQueue<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            head: 0,
            tail: 0,
            counters: TxCounters {
                fill_calls: 0,
                accepted_bytes: 0,
                short_accepts: 0,
                zero_accepts: 0,
                invalid_accepts: 0,
            },
            max_offer: 64,
            state: TxState::Ready,
        }
    }

    /// Offer limit independent of software staging capacity (N).
    /// The P4 first comparison should use 64 bytes in both variants.
    pub const fn with_max_offer(max_offer: usize) -> Result<Self, TxConfigError> {
        if max_offer == 0 || max_offer > i32::MAX as usize {
            return Err(TxConfigError::InvalidMaxOffer);
        }
        let mut queue = Self::new();
        queue.max_offer = max_offer;
        Ok(queue)
    }

    pub const fn max_offer(&self) -> usize {
        self.max_offer
    }

    pub const fn state(&self) -> TxState {
        self.state
    }

    /// Only call after the *transport* has been explicitly re-synchronized:
    /// aborting the previous frame/session and establishing fresh framing.
    /// Drops all unresolved queued bytes; never replays ambiguous data.
    /// This method does not perform link resynchronization itself.
    pub fn reset_after_resync(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.state = TxState::Ready;
    }

    pub const fn pending(&self) -> usize {
        self.tail - self.head
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn counters(&self) -> TxCounters {
        self.counters
    }

    /// Queue as many bytes as available space permits, preserving byte order.
    ///
    /// A returned count less than `input.len()` indicates backpressure when
    /// the queue is ready. A return of zero can also mean the queue is locked
    /// in `TxState::ResyncRequired`; callers must check `state()` to tell a
    /// full buffer from a locked queue. They retain all unaccepted input.
    /// Nothing is silently discarded.
    pub fn enqueue(&mut self, input: &[u8]) -> usize {
        if self.state != TxState::Ready {
            return 0;
        }
        if self.head > 0 {
            // Compact only on enqueue, never while the FIFO adapter borrows data.
            self.bytes.copy_within(self.head..self.tail, 0);
            self.tail -= self.head;
            self.head = 0;
        }
        let n = core::cmp::min(input.len(), N - self.tail);
        self.bytes[self.tail..self.tail + n].copy_from_slice(&input[..n]);
        self.tail += n;
        n
    }

    /// Single-step transfer. Never retries automatically on 0 or on error,
    /// preventing an unbounded busy-spin in a RAM-resident stub.
    /// Indeterminate outcomes permanently lock this queue until
    /// reset_after_resync() is called following EXTERNAL link resynchronization.
    pub fn pump_once<F: FifoFill>(&mut self, fifo: &mut F) -> Result<PumpResult, TxError> {
        if self.state != TxState::Ready {
            return Err(TxError::ResyncRequired);
        }
        if self.pending() == 0 {
            return Ok(PumpResult::Empty);
        }
        let offered = core::cmp::min(self.pending(), self.max_offer);
        let result = fifo.fill(&self.bytes[self.head..self.head + offered]);
        self.counters.fill_calls = self.counters.fill_calls.saturating_add(1);
        let accepted = match result {
            Ok(bytes) => bytes,
            Err(FifoFailure::NoProgress(code)) => return Err(TxError::NoProgress(code)),
            Err(FifoFailure::Indeterminate(code)) => {
                self.counters.invalid_accepts = self.counters.invalid_accepts.saturating_add(1);
                self.state = TxState::ResyncRequired;
                return Err(TxError::Indeterminate(code));
            }
        };
        if accepted > offered {
            self.counters.invalid_accepts = self.counters.invalid_accepts.saturating_add(1);
            self.state = TxState::ResyncRequired;
            return Err(TxError::InvalidAcceptance {
                offered,
                returned: accepted,
            });
        }
        if accepted == 0 {
            self.counters.zero_accepts = self.counters.zero_accepts.saturating_add(1);
            return Ok(PumpResult::Backpressure {
                remaining: self.pending(),
            });
        }
        if (accepted) < offered {
            self.counters.short_accepts = self.counters.short_accepts.saturating_add(1);
        }
        self.head += accepted;
        self.counters.accepted_bytes = self.counters.accepted_bytes.saturating_add(accepted as u64);
        let remaining = self.pending();
        if remaining == 0 {
            self.head = 0;
            self.tail = 0;
        }
        Ok(PumpResult::Progress {
            accepted: accepted,
            remaining,
        })
    }
}
