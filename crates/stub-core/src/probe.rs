//! Allocation-free diagnostic protocol and SLIP response encoding.
//!
//! This is a software-only foundation for the next USB-OTG image. The
//! qualified P4 OHAI/PING candidate does not depend on this module.
use crate::{SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_ESC_ESC};

/// Maximum user payload echoed by the diagnostic (not a transport MTU).
pub const MAX_ECHO: usize = 32;
pub const PROTOCOL_ID: &[u8] = b"IDLAB-DIAG/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeError {
    Unsupported,
    PayloadTooLong,
    OutputTooSmall,
}

/// Recognize complete decoded SLIP payloads. Neither ROM nor flash is accessed.
/// Invalid or unsupported requests never produce a partial reply.
pub fn reply(request: &[u8], output: &mut [u8]) -> Result<usize, ProbeError> {
    let response = if request == b"PING" {
        &b"PONG"[..]
    } else if request == b"INFO" {
        PROTOCOL_ID
    } else if let Some(data) = request.strip_prefix(b"ECHO") {
        if data.len() > MAX_ECHO {
            return Err(ProbeError::PayloadTooLong);
        }
        request
    } else {
        return Err(ProbeError::Unsupported);
    };
    if output.len() < response.len() {
        return Err(ProbeError::OutputTooSmall);
    }
    output[..response.len()].copy_from_slice(response);
    Ok(response.len())
}

/// Encode a complete payload with SLIP boundary and escape bytes.
///
/// Check the exact encoded size *before* modifying output. This is safe
/// for a fixed-size TX queue and never silently truncates a frame.
pub fn encode_slip(payload: &[u8], output: &mut [u8]) -> Result<usize, ProbeError> {
    let mut needed = 2usize;
    for &byte in payload {
        needed = needed.checked_add(if byte == SLIP_END || byte == SLIP_ESC { 2 } else { 1 })
            .ok_or(ProbeError::OutputTooSmall)?;
    }
    if output.len() < needed {
        return Err(ProbeError::OutputTooSmall);
    }
    let mut n = 0;
    output[n] = SLIP_END;
    n += 1;
    for &byte in payload {
        if byte == SLIP_END || byte == SLIP_ESC {
            output[n] = SLIP_ESC;
            output[n + 1] = if byte == SLIP_END { SLIP_ESC_END } else { SLIP_ESC_ESC };
            n += 2;
        } else {
            output[n] = byte;
            n += 1;
        }
    }
    output[n] = SLIP_END;
    Ok(n + 1)
}
