// Host-only peer for the actual esptool-js methods. Length prefixes carry
// decoded SLIP frames over pipes; physical USB and ROM calls are outside this test.
mod flash;
#[path = "../../targets/esp32p4/src/program.rs"]
mod program;
use flash::Nor;
use program::Download;
use std::io::{self, Read, Write};
use stub_core::{
    dispatcher::{dispatch, AfterReply, Target},
    protocol::{response, FlashReadIo, ReadError, ReadScratch},
};
struct Peer {
    nor: Nor,
    download: Option<Download>,
    hash_input: Vec<u8>,
}
#[link(name = "crypto")]
unsafe extern "C" {
    fn MD5(input: *const u8, length: usize, out: *mut u8) -> *mut u8;
}
fn hash(input: &[u8]) -> [u8; 16] {
    let mut out = [0; 16];
    unsafe {
        MD5(input.as_ptr(), input.len(), out.as_mut_ptr());
    }
    out
}
fn recv() -> io::Result<Vec<u8>> {
    let mut size = [0; 4];
    io::stdin().read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    assert!(size <= 65536);
    let mut data = vec![0; size];
    io::stdin().read_exact(&mut data)?;
    Ok(data)
}
impl Target for Peer {
    fn capacity(&self) -> u32 {
        self.nor.0.borrow().len() as u32
    }
    fn busy(&self) -> bool {
        self.download.is_some()
    }
    fn attach(&mut self) -> Result<(), u8> {
        Ok(())
    }
    fn configure(&mut self, _: [u32; 6]) -> Result<(), u8> {
        Err(0xff)
    }
    fn erase(&mut self, a: u32, n: u32) -> Result<(), u8> {
        self.nor.erase(a, n)
    }
    fn md5(&mut self, a: u32, n: u32, out: &mut [u8; 16]) -> Result<(), u8> {
        self.nor.bounds(a, n)?;
        *out = hash(&self.nor.0.borrow()[a as usize..(a + n) as usize]);
        Ok(())
    }
    fn security_info(&mut self, _: &mut [u8; 20]) -> Result<(), u8> {
        Err(0xff)
    }
    fn registers(&mut self, _: u8, _: &[u8]) -> Result<u32, u8> {
        Err(0xff)
    }
    fn begin(&mut self, op: u8, p: &[u8]) -> Result<(), u8> {
        self.download = None;
        self.download = Some(Download::begin(op, p, &self.nor)?);
        Ok(())
    }
    fn validate_data(&self, op: u8, p: &[u8], sum: u32) -> Result<(), u8> {
        self.download
            .as_ref()
            .ok_or(0xc6u8)?
            .validate_data(op, p, sum)
    }
    fn finish(&mut self, op: u8, p: &[u8]) -> Result<Option<u32>, u8> {
        self.download.take().ok_or(0xc6u8)?.finish(op, p)
    }
}
impl FlashReadIo for Peer {
    fn read_aligned(&mut self, a: u32, out: &mut [u8]) -> Result<(), ReadError> {
        out.copy_from_slice(&self.nor.0.borrow()[a as usize..a as usize + out.len()]);
        Ok(())
    }
    fn send_frame(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
        let mut w = io::stdout().lock();
        w.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
        Ok(())
    }
    fn receive_ack(&mut self) -> Result<u32, ReadError> {
        Ok(u32::from_le_bytes(
            recv()
                .unwrap()
                .try_into()
                .map_err(|_| ReadError::UnexpectedAck)?,
        ))
    }
    fn begin_digest(&mut self) {
        self.hash_input.clear()
    }
    fn update_digest(&mut self, b: &[u8]) {
        self.hash_input.extend_from_slice(b)
    }
    fn finish_digest(&mut self) -> [u8; 16] {
        hash(&self.hash_input)
    }
}
fn main() {
    let mut peer = Peer {
        nor: Nor(vec![0; 262144].into()),
        download: None,
        hash_input: Vec::new(),
    };
    let mut pending = None;
    while let Ok(frame) = recv() {
        let r = dispatch(&mut peer, &frame, &mut pending);
        let mut buf = [0; 30];
        let packet = response(&mut buf, r.opcode, r.value, &r.data[..r.length], r.status).unwrap();
        for _ in 0..r.count {
            peer.send_frame(packet).unwrap();
        }
        match r.after {
            AfterReply::Write {
                opcode,
                payload,
                checksum,
            } => {
                if let Err(code) = peer
                    .download
                    .as_mut()
                    .unwrap()
                    .data(opcode, payload, checksum, &peer.nor)
                {
                    pending = Some(code);
                    peer.download = None;
                }
            }
            AfterReply::Read(read) => read
                .transfer(&mut peer, &mut ReadScratch([0; 4100]))
                .unwrap(),
            AfterReply::Jump(_) => panic!("unexpected jump in flash scenario"),
            AfterReply::None => {}
        }
    }
}
