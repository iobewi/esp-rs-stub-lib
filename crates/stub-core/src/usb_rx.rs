//! Bounded, sequential USB CDC RX bridge for a single foreground owner.
//!
//! A ROM read may contain multiple SLIP frames. Bytes following a completed
//! frame are retained in the pump until DoubleBuffer::poll() arms the spare
//! slot. Never read the ROM while the receiver is disarmed or buffered bytes
//! remain. No ISR access, heap allocation or implicit retry.

use crate::rx::DoubleBuffer;

/// Initial comparison bound, not an asserted ROM packet/FIFO capacity.
pub const RX_BURST: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxFailure {
    /// The ROM result does not establish which bytes were consumed.
    Indeterminate(i32),
    /// Source returned a count larger than the provided slice.
    InvalidCount { offered: usize, returned: usize },
}

pub trait RxSource {
    /// Copies up to `bytes.len()` bytes into the slice. No retained borrow.
    /// An error is conservative: zero-consumption cannot be presumed.
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, RxFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxPumpResult {
    Empty,
    /// Receiver is disarmed. No ROM call and no bytes discarded.
    Paused { pending: usize },
    /// Number of bytes fetched from the source on this call. May be zero if
    /// previously buffered bytes were drained after a receiver rearm.
    Progress { received: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxPumpError {
    Source(RxFailure),
    ResyncRequired,
}

/// Single-owner, allocation-free RX pump. NOT ISR-safe.
pub struct RxPump {
    poisoned: bool,
    batch: [u8; RX_BURST],
    cursor: usize,
    end: usize,
    pub calls: u64,
    /// Bytes accepted from the source (including bytes pending in batch).
    pub received_bytes: u64,
}

impl Default for RxPump {
    fn default() -> Self {
        Self::new()
    }
}

impl RxPump {
    pub const fn new() -> Self {
        Self {
            poisoned: false,
            batch: [0; RX_BURST],
            cursor: 0,
            end: 0,
            calls: 0,
            received_bytes: 0,
        }
    }

    pub const fn requires_resync(&self) -> bool {
        self.poisoned
    }

    pub const fn pending(&self) -> usize {
        self.end - self.cursor
    }

    /// External caller must first establish a fresh framing/session and
    /// discard stale ROM RX data. This discards pending software RX data and
    /// unlocks the pump. Reset the DoubleBuffer separately.
    pub fn reset_after_resync(&mut self) {
        self.poisoned = false;
        self.cursor = 0;
        self.end = 0;
    }

    /// Drain cached bytes until a complete/error frame disarms the receiver.
    /// A disarmed receiver cannot consume additional bytes.
    fn drain<const N: usize>(&mut self, buffer: &mut DoubleBuffer<N>) {
        while self.cursor < self.end && buffer.is_armed() {
            buffer.receive(self.batch[self.cursor]);
            self.cursor += 1;
        }
        if self.cursor == self.end {
            self.cursor = 0;
            self.end = 0;
        }
    }

    /// At most one ROM source call per invocation. Process pending bytes
    /// first, and never call the ROM while RX is disarmed.
    pub fn pump_once<S: RxSource, const N: usize>(
        &mut self,
        source: &mut S,
        buffer: &mut DoubleBuffer<N>,
    ) -> Result<RxPumpResult, RxPumpError> {
        if self.poisoned {
            return Err(RxPumpError::ResyncRequired);
        }
        if !buffer.is_armed() {
            return Ok(RxPumpResult::Paused {
                pending: self.pending(),
            });
        }
        if self.pending() > 0 {
            self.drain(buffer);
            return Ok(RxPumpResult::Progress { received: 0 });
        }

        self.calls = self.calls.saturating_add(1);
        match source.read(&mut self.batch) {
            Ok(count) if count <= RX_BURST => {
                if count == 0 {
                    return Ok(RxPumpResult::Empty);
                }
                self.cursor = 0;
                self.end = count;
                self.received_bytes = self.received_bytes.saturating_add(count as u64);
                self.drain(buffer);
                Ok(RxPumpResult::Progress { received: count })
            }
            Ok(returned) => {
                self.poisoned = true;
                Err(RxPumpError::Source(RxFailure::InvalidCount {
                    offered: RX_BURST,
                    returned,
                }))
            }
            Err(err) => {
                self.poisoned = true;
                Err(RxPumpError::Source(err))
            }
        }
    }
}
