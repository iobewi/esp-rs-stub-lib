//! Sequential flash and RAM download sessions. RX frames are borrowed only during dispatch.
use super::flash::{Nor, SECTOR};
use stub_core::protocol::data_checksum;

pub const RAM_START: u32 = 0x4ff55000;
pub const RAM_END: u32 = 0x4ff60000;

pub fn word(bytes: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap())
}

unsafe extern "C" {
    fn tinfl_decompress(
        state: *mut u32,
        input: *const u8,
        available: *mut usize,
        base: *mut u8,
        next: *mut u8,
        space: *mut usize,
        flags: u32,
    ) -> i32;
}
// RV32 miniz tinfl_decompressor requires less than 12 KiB. ROM accesses aligned words.
#[repr(align(8))]
struct InflateStorage([u32; 3072]);
static mut INFLATOR: InflateStorage = InflateStorage([0; 3072]);
static mut HISTORY: [u8; 32768] = [0; 32768];

pub struct Download {
    kind: u8,
    base: u32,
    total: u32,
    done: u32,
    blocks: u32,
    seq: u32,
    block: u32,
    ended: bool,
    erased_until: u32,
}
impl Download {
    pub fn begin(kind: u8, p: &[u8], capacity: u32) -> Result<Self, u8> {
        if !(p.len() == 16 || (kind != 5 && p.len() == 20 && word(p, 16) == 0)) {
            return Err(0xc0);
        }
        let (total, blocks, block, base) = (word(p, 0), word(p, 4), word(p, 8), word(p, 12));
        if !(1..=16384).contains(&block) || (kind != 0x10 && blocks != total.div_ceil(block)) {
            return Err(0xc2);
        }
        if kind == 0x10 && (blocks == 0 || blocks.checked_mul(block).is_none()) {
            return Err(0xc2);
        }
        if kind == 5 {
            if !(RAM_START..=RAM_END).contains(&base) || total > RAM_END - base {
                return Err(0xc0);
            }
        } else {
            if base > capacity || total > capacity - base {
                return Err(0xc0);
            }
            if base % SECTOR != 0 {
                return Err(0xc0);
            }
        }
        if kind == 0x10 {
            unsafe { core::ptr::addr_of_mut!(INFLATOR.0).cast::<u32>().write(0) }
        }
        Ok(Self {
            erased_until: base,
            kind,
            base,
            total,
            done: 0,
            blocks,
            seq: 0,
            block,
            ended: false,
        })
    }
    fn output(&mut self, bytes: &[u8], flash: Option<&Nor>) -> Result<(), u8> {
        if bytes.len() > (self.total - self.done) as usize {
            return Err(0xc9);
        }
        let addr = self.base + self.done;
        if self.kind == 5 {
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) }
        } else {
            if !bytes.is_empty() {
                let flash = flash.ok_or(0xc4u8)?;
                let end = (addr + bytes.len() as u32).div_ceil(SECTOR) * SECTOR;
                if end > self.erased_until {
                    flash.erase(self.erased_until, end - self.erased_until)?;
                    self.erased_until = end;
                }
                flash.program(addr, bytes)?;
            }
        }
        self.done += bytes.len() as u32;
        Ok(())
    }
    pub fn validate_data(&self, op: u8, p: &[u8], sum: u32) -> Result<(), u8> {
        if !matches!((self.kind, op), (2, 3) | (5, 7) | (0x10, 0x11)) {
            return Err(0xc6);
        }
        if p.len() < 16
            || word(p, 0) as usize != p.len() - 16
            || word(p, 4) != self.seq
            || self.seq >= self.blocks
        {
            return Err(0xc0);
        }
        let data = &p[16..];
        if data.is_empty() || data.len() > self.block as usize {
            return Err(0xc2);
        }
        if u32::from(data_checksum(data)) != sum {
            return Err(0xc1);
        }
        if self.kind != 0x10 {
            let remaining = (self.total - self.done) as usize;
            if data.len() > remaining
                && (self.kind != 2
                    || self.seq + 1 != self.blocks
                    || data[remaining..].iter().any(|&b| b != 0xff))
            {
                return Err(0xc9);
            }
        }
        Ok(())
    }
    pub fn data(&mut self, op: u8, p: &[u8], sum: u32, flash: Option<&Nor>) -> Result<(), u8> {
        self.validate_data(op, p, sum)?;
        let mut data = &p[16..];
        if self.kind == 0x10 {
            let last = self.seq + 1 == self.blocks;
            loop {
                let pos = self.done as usize % 32768;
                let mut written = 32768 - pos;
                let mut used = data.len();
                let base = core::ptr::addr_of_mut!(HISTORY).cast::<u8>();
                let status = unsafe {
                    tinfl_decompress(
                        core::ptr::addr_of_mut!(INFLATOR.0).cast(),
                        data.as_ptr(),
                        &mut used,
                        base,
                        base.add(pos),
                        &mut written,
                        if last { 1 } else { 3 },
                    )
                };
                if status < 0 || used > data.len() || written > 32768 - pos {
                    return Err(0xc7);
                }
                self.output(
                    unsafe { core::slice::from_raw_parts(base.add(pos), written) },
                    flash,
                )?;
                data = &data[used..];
                match status {
                    0 if last && data.is_empty() => {
                        self.ended = true;
                        break;
                    }
                    1 if !last && data.is_empty() => break,
                    2 if used != 0 || written != 0 => continue,
                    _ => return Err(0xc7),
                }
            }
        } else {
            let remaining = (self.total - self.done) as usize;
            if data.len() > remaining
                && (self.kind != 2
                    || self.seq + 1 != self.blocks
                    || data[remaining..].iter().any(|&b| b != 0xff))
            {
                return Err(0xc9);
            }
            self.output(&data[..remaining.min(data.len())], flash)?;
        }
        self.seq += 1;
        Ok(())
    }
    pub fn finish(&self, op: u8, p: &[u8]) -> Result<Option<u32>, u8> {
        if !matches!((self.kind, op), (2, 4) | (5, 6) | (0x10, 0x12)) {
            return Err(0xc6);
        }
        if self.seq != self.blocks || self.done != self.total || (self.kind == 0x10 && !self.ended)
        {
            return Err(0xc8);
        }
        if self.kind == 5 {
            if p.len() != 8 {
                return Err(0xc0);
            }
            let entry = word(p, 4);
            if word(p, 0) == 0 && entry != 0 {
                if entry & 1 != 0 || entry < self.base || entry >= self.base + self.total {
                    return Err(0xc0);
                }
                return Ok(Some(entry));
            }
        } else if p.len() == 4 && word(p, 0) == 0 {
            return Err(0xff);
        } else if p.len() != 4 {
            return Err(0xc0);
        }
        Ok(None)
    }
}
