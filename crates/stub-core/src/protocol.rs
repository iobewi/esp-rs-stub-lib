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

/// Validated READ_FLASH parameters. The wire window is advisory; esptool-js
/// receives one packet and returns its cumulative byte count before the next.
#[derive(Debug, Clone, Copy)]
pub struct FlashRead {
    offset: u32,
    size: u32,
    packet_size: u32,
}

impl FlashRead {
    pub fn parse(payload: &[u8], capacity: u32) -> Result<Self, PacketError> {
        let fields: &[u8; 16] = payload.try_into().map_err(|_| PacketError::Length)?;
        let word = |start| {
            let i: usize = start;
            u32::from_le_bytes([fields[i], fields[i + 1], fields[i + 2], fields[i + 3]])
        };
        let (offset, size, packet_size) = (word(0), word(4), word(8));
        if offset > capacity
            || size > capacity - offset
            || packet_size > 4096
            || (size != 0 && packet_size == 0)
        {
            return Err(PacketError::Length);
        }
        Ok(Self {
            offset,
            size,
            packet_size,
        })
    }

    /// Call after sending the command success response and releasing its RX frame.
    /// The target supplies aligned storage outside the interrupt stack.
    pub fn transfer(
        self,
        io: &mut impl FlashReadIo,
        scratch: &mut ReadScratch,
    ) -> Result<(), ReadError> {
        io.begin_digest();
        let mut acknowledged = 0;
        while acknowledged < self.size {
            let address = self.offset + acknowledged;
            let count = self.packet_size.min(self.size - acknowledged);
            let skip = (address & 3) as usize;
            let words = (skip + count as usize).div_ceil(4);
            io.read_aligned(address & !3, &mut scratch.0[..words * 4])?;
            let data = &scratch.0[skip..skip + count as usize];
            io.update_digest(data);
            io.send_frame(data)?;
            let expected = acknowledged + count;
            if io.receive_ack()? != expected {
                return Err(ReadError::UnexpectedAck);
            }
            acknowledged = expected;
        }
        let digest = io.finish_digest();
        io.send_frame(&digest)
    }
}

#[repr(align(4))]
pub struct ReadScratch(pub [u8; 4100]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    Flash,
    Transport,
    UnexpectedAck,
}

pub trait FlashReadIo {
    fn read_aligned(&mut self, address: u32, output: &mut [u8]) -> Result<(), ReadError>;
    fn send_frame(&mut self, payload: &[u8]) -> Result<(), ReadError>;
    /// Accept exactly four little-endian bytes from one decoded SLIP frame.
    fn receive_ack(&mut self) -> Result<u32, ReadError>;
    fn begin_digest(&mut self);
    fn update_digest(&mut self, bytes: &[u8]);
    fn finish_digest(&mut self) -> [u8; 16];
}
