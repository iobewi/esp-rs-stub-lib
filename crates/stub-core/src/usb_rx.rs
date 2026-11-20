//! Bounded, sequential USB CDC RX bridge for a single foreground owner.
//!
//! This module never installs an ISR or touches ROM directly. A source may
//! supply up to RX_BURST bytes per call. All bytes are delivered in order to
//! DoubleBuffer, which must remain exclusively borrowed by the caller.
//! Unexpected RX reports poison the bridge until external resynchronization.

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
    pub calls: u64,
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
            calls: 0,
            received_bytes: 0,
        }
    }

    pub const fn requires_resync(&self) -> bool {
        self.poisoned
    }

    /// External caller must first establish a fresh framing/session and
    /// discard stale ROM RX data. This call only unlocks the software pump.
    /// The DoubleBuffer is reset separately with reset_after_resync().
    pub fn reset_after_resync(&mut self) {
        self.poisoned = false;
    }

    /// One source call maximum per invocation, no blocking retry.
    pub fn pump_once<S: RxSource, const N: usize>(
        &mut self,
        source: &mut S,
        buffer: &mut DoubleBuffer<N>,
    ) -> Result<RxPumpResult, RxPumpError> {
        if self.poisoned {
            return Err(RxPumpError::ResyncRequired);
        }
        let mut batch = [0u8; RX_BURST];
        self.calls = self.calls.saturating_add(1);
        match source.read(&mut batch) {
            Ok(count) if count <= RX_BURST => {
                if count == 0 {
                    return Ok(RxPumpResult::Empty);
                }
                for byte in batch[..count].iter().copied() {
                    buffer.receive(byte);
                }
                self.received_bytes = self.received_bytes.saturating_add(count as u64);
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
