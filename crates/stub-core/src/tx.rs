//! Lossless, nonblocking staging for a FIFO that reports accepted byte counts.
//!
//! A call to `pump_once` makes *at most one* ROM/FIFO call. If only part of the
//! buffer is accepted, the remaining bytes are retained, in order. This
//! prevents the data loss caused by discarding a buffer after partial fill.
//! No USB hardware access or ISR synchronization occurs in this module.

/// An initial experimental staging size, not a claim about ROM FIFO capacity.
/// The platform should benchmark it against 64-byte staging on real hardware.
pub const USB_TX_STAGING_BYTES: usize = 4096;

/// Models the signed `int` result of Espressif's CDC ACM ROM FIFO fill.
///
/// The return value is the number of bytes accepted, potentially zero. A
/// negative return or a result larger than the offered slice is an error.
/// Implementations must not retain the slice after returning.
pub trait FifoFill {
    fn fill(&mut self, bytes: &[u8]) -> i32;
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
    /// A negative return code from the ROM adapter. Data is retained.
    RomFailure(i32),
    /// The adapter claimed to accept more bytes than were offered.
    InvalidAcceptance { offered: usize, returned: i32 },
}

/// Sequential TX owner. NOT safe to share between interrupt and foreground
/// code without a platform-specific critical section / ownership protocol.
pub struct TxQueue<const N: usize> {
    bytes: [u8; N],
    head: usize,
    tail: usize,
    counters: TxCounters,
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
        }
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
    /// A returned count less than `input.len()` is explicit backpressure; the
    /// caller retains and retries the *unaccepted input* itself. Nothing is
    /// silently discarded.
    pub fn enqueue(&mut self, input: &[u8]) -> usize {
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
    pub fn pump_once<F: FifoFill>(&mut self, fifo: &mut F) -> Result<PumpResult, TxError> {
        if self.pending() == 0 {
            return Ok(PumpResult::Empty);
        }
        // ROM function takes an int length; cap to avoid an ABI truncation.
        let offered = core::cmp::min(self.pending(), i32::MAX as usize);
        let accepted = fifo.fill(&self.bytes[self.head..self.head + offered]);
        self.counters.fill_calls = self.counters.fill_calls.saturating_add(1);

        if accepted < 0 {
            self.counters.invalid_accepts = self.counters.invalid_accepts.saturating_add(1);
            return Err(TxError::RomFailure(accepted));
        }
        if accepted as usize > offered {
            self.counters.invalid_accepts = self.counters.invalid_accepts.saturating_add(1);
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
        if (accepted as usize) < offered {
            self.counters.short_accepts = self.counters.short_accepts.saturating_add(1);
        }
        self.head += accepted as usize;
        self.counters.accepted_bytes = self.counters.accepted_bytes.saturating_add(accepted as u64);
        let remaining = self.pending();
        if remaining == 0 {
            self.head = 0;
            self.tail = 0;
        }
        Ok(PumpResult::Progress {
            accepted: accepted as usize,
            remaining,
        })
    }
}
