#![no_std]
#![no_main]

include!("../usb_runtime.inc.rs");

#[path = "../flash.rs"]
mod flash;
#[path = "../program.rs"]
mod program;
use program::{word, Download};
use stub_core::protocol::{response, FlashRead, FlashReadIo, ReadError, ReadScratch, Request};
use stub_core::slip::{send_frame, TxSink};

unsafe extern "C" {
    fn GetSecurityInfoProc(msg: *mut i32, error: *mut i32, buf: *mut u8) -> u32;
    fn MD5Init(ctx: *mut u32);
    fn MD5Update(ctx: *mut u32, bytes: *const u8, len: u32);
    fn MD5Final(digest: *mut u8, ctx: *mut u32);

}

const FRAME: usize = stub_core::rx::FRAME_BUFFER_SIZE;
static mut FRAMES: DoubleBuffer<FRAME> = DoubleBuffer::new();
static mut SCRATCH: ReadScratch = ReadScratch([0; 4100]);

struct Session {
    nor: flash::Nor,
    port: P4RomCdcTx,
    mask: CdcRxMask,
    coord: RxIrqCoordinator,
    pump: RxPump,
    ticks: u32,
    digest: [u32; 22],
    tx: [u8; 64],
    used: usize,
}

impl TxSink for Session {
    type Error = ReadError;
    fn write_byte(&mut self, byte: u8) -> Result<(), ReadError> {
        self.tx[self.used] = byte;
        self.used += 1;
        if self.used == self.tx.len() {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ReadError> {
        if self.used != 0
            && !send(
                &mut self.port,
                &mut self.coord,
                &mut self.mask,
                self.ticks,
                &self.tx[..self.used],
            )
        {
            return Err(ReadError::Transport);
        }
        self.used = 0;
        Ok(())
    }
}

impl Session {
    fn receive(
        &mut self,
        frames: &mut DoubleBuffer<FRAME>,
        awaiting_ack: bool,
    ) -> Result<usize, ReadError> {
        let start = cycle_count();
        loop {
            if awaiting_ack && cycle_count().wrapping_sub(start) >= self.ticks * 3_000_000 {
                return Err(ReadError::Transport);
            }
            let state = without_local_interrupts(|| {
                self.coord.service_once(
                    &SIGNAL,
                    &mut self.mask,
                    &mut self.port,
                    &mut self.pump,
                    frames,
                )
            });
            match state {
                Err(_) => return Err(ReadError::Transport),
                Ok(ServiceResult::LinkChange) => {
                    if without_local_interrupts(|| {
                        self.coord.resolve_line_change(&SIGNAL, &mut self.mask)
                    }) == LineDecision::ResyncRequired
                    {
                        return Err(ReadError::Transport);
                    }
                }
                _ => {}
            }
            match frames.poll() {
                Poll::Ready { index, .. } => return Ok(index),
                Poll::Error { .. } => frames.release_selected(),
                Poll::Idle => {}
            }
        }
    }
}

impl FlashReadIo for Session {
    fn read_aligned(&mut self, address: u32, out: &mut [u8]) -> Result<(), ReadError> {
        self.nor.read(address, out).map_err(|_| ReadError::Flash)
    }
    fn send_frame(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
        send_frame(bytes, self)
    }
    fn receive_ack(&mut self) -> Result<u32, ReadError> {
        // The dispatcher releases its command frame before invoking transfer.
        let frames = unsafe { &mut *ptr::addr_of_mut!(FRAMES) };
        let index = self.receive(frames, true)?;
        let ack = frames
            .frame(index)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes);
        frames.release_selected();
        ack.ok_or(ReadError::UnexpectedAck)
    }
    fn begin_digest(&mut self) {
        unsafe { MD5Init(self.digest.as_mut_ptr()) }
    }
    fn update_digest(&mut self, data: &[u8]) {
        unsafe { MD5Update(self.digest.as_mut_ptr(), data.as_ptr(), data.len() as u32) }
    }
    fn finish_digest(&mut self) -> [u8; 16] {
        let mut result = [0; 16];
        unsafe { MD5Final(result.as_mut_ptr(), self.digest.as_mut_ptr()) };
        result
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() -> ! {
    let Some((port, mask)) = attach() else { stop() };
    unsafe { asm!("csrsi mstatus, 8", options(nostack)) };
    let ticks = unsafe { esp_rom_get_cpu_ticks_per_us() };
    if !(1..=1000).contains(&ticks) {
        stop_transport()
    }
    let mut session = Session {
        nor: flash::Nor::attach(ticks).unwrap_or_else(|_| stop_transport()),
        port,
        mask,
        ticks,
        coord: RxIrqCoordinator::new(),
        pump: RxPump::new(),
        digest: [0; 22],
        tx: [0; 64],
        used: 0,
    };
    if session.send_frame(b"OHAI").is_err() {
        stop_transport()
    }
    let mut pending_error = None;
    let mut download: Option<Download> = None;
    loop {
        let frames = unsafe { &mut *ptr::addr_of_mut!(FRAMES) };
        let Ok(index) = session.receive(frames, false) else {
            stop_transport()
        };
        let frame = frames.frame(index).unwrap_or(&[]);
        let mut opcode = frame.get(1).copied().unwrap_or(0);
        let mut status = [0xc0, 0];
        #[repr(align(4))]
        struct ReplyData([u8; 20]);
        let mut storage = ReplyData([0; 20]);
        let digest = &mut storage.0;
        let mut digest_len = 0;
        let mut value = 0;
        let mut entry = None;
        let mut post_write = None;
        let mut repeats = 1;
        let mut read = None;
        if let Ok(request) = Request::parse(frame) {
            opcode = request.opcode;
            let p = request.payload;
            let result = (|| -> Result<(), u8> {
                if let Some(code) = pending_error.take() {
                    return Err(code);
                }
                if download.is_some() && !matches!(opcode,2..=7|0x10..=0x12) {
                    return Err(0xc6);
                }
                match opcode {
                    8 if p.len() == 36 => repeats = 8,
                    0xd2 => {
                        read = Some(FlashRead::parse(p, session.nor.capacity).map_err(|_| 0xc0u8)?)
                    }
                    0x14 if p.is_empty() => {
                        if unsafe {
                            GetSecurityInfoProc(
                                ptr::null_mut(),
                                ptr::null_mut(),
                                digest.as_mut_ptr(),
                            )
                        } != 0
                        {
                            return Err(0xc4);
                        }
                        digest_len = 20;
                    }
                    0x13 if p.len() == 16 => {
                        let mut address = word(p, 0);
                        let mut remaining = word(p, 4);
                        session.nor.bounds(address, remaining)?;
                        session.begin_digest();
                        let scratch = unsafe { &mut *ptr::addr_of_mut!(SCRATCH) };
                        while remaining > 0 {
                            let count = remaining.min(4096);
                            let block = &mut scratch.0[..count as usize];
                            session.nor.read(address, block)?;
                            session.update_digest(block);
                            address += count;
                            remaining -= count;
                        }
                        digest[..16].copy_from_slice(&session.finish_digest());
                        digest_len = 16;
                    }
                    0x0d if p.len() == 4 && word(p, 0) == 0 => {
                        session.nor = flash::Nor::attach(ticks)?;
                    }
                    0x0b if p.len() == 24 => session
                        .nor
                        .configure(core::array::from_fn(|i| word(p, 4 * i)))?,
                    0xd0 if p.is_empty() => session.nor.erase(0, session.nor.capacity)?,
                    0xd1 if p.len() == 8 => session.nor.erase(word(p, 0), word(p, 4))?,
                    2 | 5 | 0x10 => {
                        download = None;
                        download = Some(Download::begin(opcode, p, &session.nor)?);
                    }
                    3 | 7 | 0x11 => {
                        let state = download.as_ref().ok_or(0xc6u8)?;
                        state.validate_data(opcode, p, request.checksum)?;
                        post_write = Some((opcode, p, request.checksum));
                    }
                    4 | 6 | 0x12 => {
                        entry = download.take().ok_or(0xc6u8)?.finish(opcode, p)?;
                    }
                    10 if p.len() == 4 => {
                        let addr = word(p, 0);
                        if !reg_allowed(addr, false) {
                            return Err(0xc0);
                        }
                        value = unsafe { ptr::read_volatile(addr as *const u32) };
                    }
                    9 if !p.is_empty() && p.len() % 16 == 0 => {
                        for item in p.chunks_exact(16) {
                            if !reg_allowed(word(item, 0), true) || word(item, 12) > 1000 {
                                return Err(0xc0);
                            }
                        }
                        for item in p.chunks_exact(16) {
                            let addr = word(item, 0) as *mut u32;
                            let mask = word(item, 8);
                            unsafe {
                                ptr::write_volatile(
                                    addr,
                                    (ptr::read_volatile(addr) & !mask) | (word(item, 4) & mask),
                                )
                            };
                            let start = cycle_count();
                            while cycle_count().wrapping_sub(start) < word(item, 12) * ticks {}
                        }
                    }
                    0x0f | 0xd3 | 0xd4 => return Err(0xff),
                    _ => return Err(0xc3),
                }
                Ok(())
            })();
            status = [result.err().unwrap_or(0), 0];
        }
        let mut buffer = [0; 30];
        let reply = response(&mut buffer, opcode, value, &digest[..digest_len], status).unwrap();
        for _ in 0..repeats {
            if session.send_frame(reply).is_err() {
                stop_transport()
            }
        }
        if let Some((op, p, sum)) = post_write {
            if let Some(state) = download.as_mut() {
                if let Err(code) = state.data(op, p, sum, &session.nor) {
                    download = None;
                    pending_error = Some(code);
                }
            }
        }
        frames.release_selected();
        if let Some(address) = entry {
            unsafe {
                esp_rom_isr_mask(USB_IRQ_MASK);
                asm!("fence.i");
                let run: unsafe extern "C" fn() -> ! = core::mem::transmute(address);
                run()
            }
        }
        if let Some(read) = read {
            let scratch = unsafe { &mut *ptr::addr_of_mut!(SCRATCH) };
            if read.transfer(&mut session, scratch).is_err() {
                stop_transport()
            }
        }
    }
}

fn reg_allowed(addr: u32, write: bool) -> bool {
    addr % 4 == 0
        && ((0x5008d000..=0x5008d0bc).contains(&addr)
            || (!write && (0x5012d030..=0x5012d060).contains(&addr)))
}
