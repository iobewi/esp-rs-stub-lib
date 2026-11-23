//! Dispatch one decoded esptool command. The caller owns the RX frame until
//! its response has been sent and any deferred write has consumed the payload.
use crate::protocol::{FlashRead, Request};

pub trait Target {
    fn capacity(&self) -> u32;
    fn busy(&self) -> bool;
    fn attach(&mut self) -> Result<(), u8>;
    fn configure(&mut self, geometry: [u32; 6]) -> Result<(), u8>;
    fn erase(&mut self, address: u32, size: u32) -> Result<(), u8>;
    fn md5(&mut self, address: u32, size: u32, out: &mut [u8; 16]) -> Result<(), u8>;
    fn security_info(&mut self, out: &mut [u8; 20]) -> Result<(), u8>;
    fn registers(&mut self, op: u8, p: &[u8]) -> Result<u32, u8>;
    fn begin(&mut self, op: u8, p: &[u8]) -> Result<(), u8>;
    fn validate_data(&self, op: u8, p: &[u8], sum: u32) -> Result<(), u8>;
    fn finish(&mut self, op: u8, p: &[u8]) -> Result<Option<u32>, u8>;
}

pub enum AfterReply<'a> {
    None,
    Read(FlashRead),
    Write {
        opcode: u8,
        payload: &'a [u8],
        checksum: u32,
    },
    Jump(u32),
}

#[repr(align(4))]
pub struct Reply<'a> {
    pub data: [u8; 20],
    pub length: usize,
    pub opcode: u8,
    pub value: u32,
    pub status: [u8; 2],
    pub count: u8,
    pub after: AfterReply<'a>,
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub fn dispatch<'a>(
    target: &mut impl Target,
    frame: &'a [u8],
    pending: &mut Option<u8>,
) -> Reply<'a> {
    let mut reply = Reply {
        data: [0; 20],
        length: 0,
        opcode: frame.get(1).copied().unwrap_or(0),
        value: 0,
        status: [0, 0],
        count: 1,
        after: AfterReply::None,
    };
    let result = (|| {
        let req = Request::parse(frame).map_err(|_| 0xc0u8)?;
        if let Some(error) = pending.take() {
            return Err(error);
        }
        let op = req.opcode;
        let p = req.payload;
        if target.busy() && !matches!(op,2..=7|0x10..=0x12) {
            return Err(0xc6);
        }
        match (op, p.len()) {
            (8, 36) => reply.count = 8,
            (0xd2, 16) => {
                reply.after =
                    AfterReply::Read(FlashRead::parse(p, target.capacity()).map_err(|_| 0xc0u8)?)
            }
            (0x14, 0) => {
                target.security_info(&mut reply.data)?;
                reply.length = 20;
            }
            (0x13, 16) => {
                target.md5(
                    word(p, 0),
                    word(p, 4),
                    (&mut reply.data[..16]).try_into().unwrap(),
                )?;
                reply.length = 16;
            }
            (0x0d, 4) if word(p, 0) == 0 => target.attach()?,
            (0x0b, 24) => target.configure(core::array::from_fn(|i| word(p, 4 * i)))?,
            (0xd0, 0) => target.erase(0, target.capacity())?,
            (0xd1, 8) => target.erase(word(p, 0), word(p, 4))?,
            (2 | 5 | 0x10, _) => target.begin(op, p)?,
            (3 | 7 | 0x11, _) => {
                target.validate_data(op, p, req.checksum)?;
                reply.after = AfterReply::Write {
                    opcode: op,
                    payload: p,
                    checksum: req.checksum,
                };
            }
            (4 | 6 | 0x12, _) => {
                if let Some(entry) = target.finish(op, p)? {
                    reply.after = AfterReply::Jump(entry);
                }
            }
            (10, 4) | (9, _) if !p.is_empty() && (op == 10 || p.len() % 16 == 0) => {
                reply.value = target.registers(op, p)?
            }
            (0x0f | 0xd3 | 0xd4, _) => return Err(0xff),
            (8..=11 | 0xd | 0x13 | 0x14 | 0xd0..=0xd2, _) => return Err(0xc0),
            _ => return Err(0xc3),
        }
        Ok(())
    })();
    reply.status[0] = result.err().unwrap_or(0);
    reply
}
