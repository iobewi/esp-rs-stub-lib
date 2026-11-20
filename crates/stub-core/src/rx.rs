//! Two fixed-size receive slots, with zero heap allocations.
//!
//! The C reference switches the RX slot when a complete/error frame is
//! pending and never releases the currently filling slot. This module models
//! those transitions *sequentially*. It is NOT an ISR-safe shared object.
//! A later platform layer must establish synchronization between producer
//! (interrupt/DMA) and consumer before sharing this memory across contexts.

use crate::slip::{DecodeEvent, SlipDecoder};

/// Mirrors 8-byte header + 0x4000-byte block + 0xFF padding in Espressif.
pub const FRAME_BUFFER_SIZE: usize = 8 + 0x4000 + 0xff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotState {
    Empty,
    Complete(usize),
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Poll {
    Idle,
    Ready { index: usize, len: usize },
    Error { index: usize },
}

pub struct DoubleBuffer<const N: usize> {
    bytes: [[u8; N]; 2],
    slots: [SlotState; 2],
    rx: usize,
    selected: Option<usize>,
    armed: bool,
    len: usize,
    decoder: SlipDecoder,
}

impl<const N: usize> Default for DoubleBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DoubleBuffer<N> {
    /// Storage is embedded in the struct. Platform code should place large
    /// instances in static RAM, not on a small interrupt stack.
    pub const fn new() -> Self {
        Self {
            bytes: [[0; N]; 2],
            slots: [SlotState::Empty; 2],
            rx: 0,
            selected: None,
            armed: true,
            len: 0,
            decoder: SlipDecoder::new(),
        }
    }

    /// Feed one byte from a sequential test or an exclusive RX adapter.
    ///
    /// Bytes are discarded while no slot is armed. A fresh C0 is required
    /// after rearming; incoming data is never treated as an implicit start.
    pub fn receive(&mut self, byte: u8) {
        match self.decoder.push(byte, self.armed) {
            DecodeEvent::None => {}
            DecodeEvent::Byte(decoded) => {
                if self.len < N {
                    self.bytes[self.rx][self.len] = decoded;
                    self.len += 1;
                } else {
                    self.slots[self.rx] = SlotState::Error;
                    self.armed = false;
                    self.decoder.reset();
                }
            }
            DecodeEvent::Complete => {
                self.slots[self.rx] = SlotState::Complete(self.len);
                self.armed = false;
                self.len = 0;
            }
            DecodeEvent::InvalidEscape => {
                self.slots[self.rx] = SlotState::Error;
                self.armed = false;
                self.len = 0;
            }
        }
    }

    /// Select the pending processing frame (errors take priority, then
    /// the other slot), and rearm a spare RX slot if one is available.
    /// Repeated polling is stable only while no receive/release operation
    /// changes the pending slots. A new RX error can take priority over a
    /// previously selected complete frame; each call updates the selected
    /// slot, so release_selected() affects the *most recent* selection.
    pub fn poll(&mut self) -> Poll {
        let other = 1 - self.rx;
        let selected = if self.slots[other] == SlotState::Error {
            Some(other)
        } else if self.slots[self.rx] == SlotState::Error {
            Some(self.rx)
        } else if matches!(self.slots[other], SlotState::Complete(_)) {
            Some(other)
        } else if matches!(self.slots[self.rx], SlotState::Complete(_)) {
            Some(self.rx)
        } else {
            None
        };
        self.selected = selected;

        // Equivalent to frame_buffer_acquire + slip_rearm in C.
        if self.slots[self.rx] != SlotState::Empty && self.slots[other] == SlotState::Empty {
            self.rx = other;
            self.armed = true;
            self.len = 0;
            self.decoder.reset();
        } else if self.slots[self.rx] == SlotState::Empty && !self.armed {
            self.armed = true;
            self.len = 0;
            self.decoder.reset();
        }

        match selected {
            None => Poll::Idle,
            Some(index) => match self.slots[index] {
                SlotState::Complete(len) => Poll::Ready { index, len },
                SlotState::Error => Poll::Error { index },
                SlotState::Empty => Poll::Idle,
            },
        }
    }

    /// Read only a completed frame. The borrow prevents receive/release from
    /// being called while a frame slice is alive in sequential use.
    pub fn frame(&self, index: usize) -> Option<&[u8]> {
        if index >= 2 {
            return None;
        }
        match self.slots[index] {
            SlotState::Complete(len) => Some(&self.bytes[index][..len]),
            _ => None,
        }
    }

    /// Release only the last selected processing slot. Idempotent; never
    /// clears the RX slot while it is actively filling.
    pub fn release_selected(&mut self) {
        if let Some(index) = self.selected.take() {
            if index != self.rx || !self.armed {
                self.slots[index] = SlotState::Empty;
            }
        }
    }

    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    pub const fn rx_index(&self) -> usize {
        self.rx
    }

    pub fn slot_state(&self, index: usize) -> Option<SlotState> {
        self.slots.get(index).copied()
    }
}
