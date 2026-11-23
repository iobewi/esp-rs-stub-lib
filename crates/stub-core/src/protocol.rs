//! Espressif request and response packets inside a decoded SLIP frame.

#[derive(Debug, PartialEq, Eq)]
pub enum PacketError {
    Truncated,
    Direction,
    Length,
    OutputCapacity,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub opcode: u8,
    pub checksum: u32,
    pub payload: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn parse(frame: &'a [u8]) -> Result<Self, PacketError> {
        let header = frame.get(..8).ok_or(PacketError::Truncated)?;
        if header[0] != 0 {
            return Err(PacketError::Direction);
        }
        let payload = &frame[8..];
        if usize::from(u16::from_le_bytes([header[2], header[3]])) != payload.len() {
            return Err(PacketError::Length);
        }
        Ok(Self {
            opcode: header[1],
            checksum: u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
            payload,
        })
    }
}

pub fn data_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0xef, |checksum, byte| checksum ^ byte)
}

/// Encode a response with its two status bytes. Errors leave the output untouched.
pub fn response<'a>(
    output: &'a mut [u8],
    opcode: u8,
    value: u32,
    data: &[u8],
    status: [u8; 2],
) -> Result<&'a [u8], PacketError> {
    let length = u16::try_from(data.len())
        .ok()
        .and_then(|n| n.checked_add(2))
        .ok_or(PacketError::Length)?;
    let packet = output
        .get_mut(..8 + usize::from(length))
        .ok_or(PacketError::OutputCapacity)?;
    packet[0] = 1;
    packet[1] = opcode;
    packet[2..4].copy_from_slice(&length.to_le_bytes());
    packet[4..8].copy_from_slice(&value.to_le_bytes());
    let end = 8 + data.len();
    packet[8..end].copy_from_slice(data);
    packet[end..].copy_from_slice(&status);
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_trailing_bytes() {
        assert_eq!(Request::parse(&[0; 9]), Err(PacketError::Length));
    }

    #[test]
    fn read_register_response() {
        let mut out = [0; 10];
        assert_eq!(
            response(&mut out, 10, 0x12345678, &[], [0, 0]).unwrap(),
            &[1, 10, 2, 0, 0x78, 0x56, 0x34, 0x12, 0, 0]
        );
    }
}
