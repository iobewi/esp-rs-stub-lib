//! Bounded, allocation-free SLIP codec for byte-oriented transports.
//!
//! The decoder mirrors Espressif's framing rules: C0 starts a frame, zero
//! length frames are ignored, C0/DB are escaped, malformed escapes are errors.
//! UART/USB integration and interrupt synchronization are outside this module.

use crate::{SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_ESC_ESC};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeState {
    NoFrame,
    InFrame,
    Escaping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeEvent {
    None,
    Byte(u8),
    Complete,
    InvalidEscape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlipDecoder {
    state: DecodeState,
    has_data: bool,
}

impl Default for SlipDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SlipDecoder {
    pub const fn new() -> Self {
        Self {
            state: DecodeState::NoFrame,
            has_data: false,
        }
    }

    pub const fn state(&self) -> DecodeState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = DecodeState::NoFrame;
        self.has_data = false;
    }

    pub fn push(&mut self, byte: u8, armed: bool) -> DecodeEvent {
        if !armed {
            self.reset();
            return DecodeEvent::None;
        }
        match self.state {
            DecodeState::NoFrame => {
                if byte == SLIP_END {
                    self.has_data = false;
                    self.state = DecodeState::InFrame;
                }
                DecodeEvent::None
            }
            DecodeState::InFrame => match byte {
                SLIP_END => {
                    self.state = DecodeState::NoFrame;
                    if self.has_data {
                        self.has_data = false;
                        DecodeEvent::Complete
                    } else {
                        // Espressif C returns to NO_FRAME on an empty frame.
                        // Following non-delimiter bytes are not frame data.
                        DecodeEvent::None
                    }
                }
                SLIP_ESC => {
                    self.state = DecodeState::Escaping;
                    DecodeEvent::None
                }
                b => {
                    self.has_data = true;
                    DecodeEvent::Byte(b)
                }
            },
            DecodeState::Escaping => {
                self.state = DecodeState::InFrame;
                match byte {
                    SLIP_ESC_END => {
                        self.has_data = true;
                        DecodeEvent::Byte(SLIP_END)
                    }
                    SLIP_ESC_ESC => {
                        self.has_data = true;
                        DecodeEvent::Byte(SLIP_ESC)
                    }
                    _ => {
                        self.reset();
                        DecodeEvent::InvalidEscape
                    }
                }
            }
        }
    }
}

/// A sink that propagates byte-send failures rather than ignoring them.
/// A partially transmitted frame must be recovered at the transport layer.
pub trait TxSink {
    type Error;
    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
}

/// Encode one frame, including delimiters, then flush exactly once.
/// Works for empty payloads (C0 C0), as the Espressif C encoder does.
pub fn send_frame<S: TxSink>(payload: &[u8], sink: &mut S) -> Result<(), S::Error> {
    sink.write_byte(SLIP_END)?;
    for &byte in payload {
        match byte {
            SLIP_END => {
                sink.write_byte(SLIP_ESC)?;
                sink.write_byte(SLIP_ESC_END)?;
            }
            SLIP_ESC => {
                sink.write_byte(SLIP_ESC)?;
                sink.write_byte(SLIP_ESC_ESC)?;
            }
            other => sink.write_byte(other)?,
        }
    }
    sink.write_byte(SLIP_END)?;
    sink.flush()
}
