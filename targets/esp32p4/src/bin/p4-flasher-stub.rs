#![no_std]
#![no_main]

include!("../usb_runtime.inc.rs");

#[path = "../flash.rs"]
mod flash;
#[path = "../program.rs"]
mod program;
use program::{word, Download};
use stub_core::dispatcher::{AfterReply, Target};
use stub_core::protocol::{response, FlashReadIo, ReadError, ReadScratch};
use stub_core::slip::{send_frame, TxSink};

unsafe extern "C" {
    fn GetSecurityInfoProc(msg: *mut i32, error: *mut i32, buf: *mut u8) -> u32;
    fn MD5Init(ctx: *mut u32);
    fn MD5Update(ctx: *mut u32, bytes: *const u8, len: u32);
    fn MD5Final(digest: *mut u8, ctx: *mut u32);

}

const FRAME: usize = stub_core::rx::FRAME_BUFFER_SIZE;
static mut TX: TxQueue<4096> = TxQueue::new();
type Frames = DoubleBuffer<FRAME>;
static mut FRAMES: core::mem::MaybeUninit<Frames> = core::mem::MaybeUninit::uninit();
static mut SCRATCH: ReadScratch = ReadScratch([0; 4100]);

struct Session {
    nor: flash::Nor,
    download: Option<Download>,
    port: P4RomCdcTx,
    mask: CdcRxMask,
    coord: RxIrqCoordinator,
    pump: RxPump,
    ticks: u32,
    digest: [u32; 22],
    tx: &'static mut TxQueue<4096>,
}

impl TxSink for Session {
    type Error = ReadError;
    fn write_byte(&mut self, byte: u8) -> Result<(), ReadError> {
        if self.tx.pending() == self.tx.capacity() {
            self.flush()?;
        }
        if self.tx.enqueue(core::slice::from_ref(&byte)) != 1 {
            return Err(ReadError::Transport);
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ReadError> {
        if drain_tx(
            self.tx,
            &mut self.port,
            &mut self.coord,
            &mut self.mask,
            self.ticks,
        ) {
            Ok(())
        } else {
            Err(ReadError::Transport)
        }
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
        let frames = unsafe { &mut *ptr::addr_of_mut!(FRAMES).cast::<Frames>() };
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
    let first_cycle = cycle_count();
    if !(1..=1000).contains(&ticks) || cycle_count() == first_cycle {
        stop_transport()
    }
    Frames::init_in(unsafe { &mut *ptr::addr_of_mut!(FRAMES) });
    let mut session = Session {
        download: None,
        nor: flash::Nor::attach(ticks).unwrap_or_else(|_| stop_transport()),
        port,
        mask,
        ticks,
        coord: RxIrqCoordinator::new(),
        pump: RxPump::new(),
        digest: [0; 22],
        tx: unsafe { &mut *ptr::addr_of_mut!(TX) },
    };
    if session.send_frame(b"OHAI").is_err() {
        stop_transport()
    }
    let mut pending_error = None;
    loop {
        let frames = unsafe { &mut *ptr::addr_of_mut!(FRAMES).cast::<Frames>() };
        let Ok(index) = session.receive(frames, false) else {
            stop_transport()
        };
        let frame = frames.frame(index).unwrap_or(&[]);
        let reply = stub_core::dispatcher::dispatch(&mut session, frame, &mut pending_error);
        let mut buffer = [0; 30];
        let packet = response(
            &mut buffer,
            reply.opcode,
            reply.value,
            &reply.data[..reply.length],
            reply.status,
        )
        .unwrap();
        for _ in 0..reply.count {
            if session.send_frame(packet).is_err() {
                stop_transport()
            }
        }
        if let AfterReply::Write {
            opcode,
            payload,
            checksum,
        } = reply.after
        {
            if let Err(error) =
                session
                    .download
                    .as_mut()
                    .unwrap()
                    .data(opcode, payload, checksum, &session.nor)
            {
                session.download = None;
                pending_error = Some(error);
            }
        }
        let after = match reply.after {
            AfterReply::Jump(addr) => AfterReply::Jump(addr),
            AfterReply::Read(read) => AfterReply::Read(read),
            _ => AfterReply::None,
        };
        frames.release_selected();
        if let AfterReply::Jump(address) = after {
            unsafe {
                esp_rom_isr_mask(USB_IRQ_MASK);
                asm!("fence.i");
                let run: unsafe extern "C" fn() -> ! = core::mem::transmute(address);
                run()
            }
        }
        if let AfterReply::Read(read) = after {
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

impl Target for Session {
    fn capacity(&self) -> u32 {
        self.nor.capacity
    }
    fn busy(&self) -> bool {
        self.download.is_some()
    }
    fn attach(&mut self) -> Result<(), u8> {
        self.nor = flash::Nor::attach(self.ticks)?;
        Ok(())
    }
    fn configure(&mut self, p: [u32; 6]) -> Result<(), u8> {
        self.nor.configure(p)
    }
    fn erase(&mut self, addr: u32, len: u32) -> Result<(), u8> {
        self.nor.erase(addr, len)
    }
    fn md5(&mut self, mut addr: u32, mut remaining: u32, out: &mut [u8; 16]) -> Result<(), u8> {
        self.nor.bounds(addr, remaining)?;
        self.begin_digest();
        let scratch = unsafe { &mut *ptr::addr_of_mut!(SCRATCH) };
        while remaining > 0 {
            let len = remaining.min(4096);
            let bytes = &mut scratch.0[..len as usize];
            self.nor.read(addr, bytes)?;
            self.update_digest(bytes);
            addr += len;
            remaining -= len;
        }
        *out = self.finish_digest();
        Ok(())
    }
    fn security_info(&mut self, out: &mut [u8; 20]) -> Result<(), u8> {
        #[repr(align(4))]
        struct Words([u8; 20]);
        let mut data = Words([0; 20]);
        if unsafe { GetSecurityInfoProc(ptr::null_mut(), ptr::null_mut(), data.0.as_mut_ptr()) }
            != 0
        {
            return Err(0xc4);
        }
        *out = data.0;
        Ok(())
    }
    fn registers(&mut self, op: u8, p: &[u8]) -> Result<u32, u8> {
        if op == 10 {
            let addr = word(p, 0);
            if !reg_allowed(addr, false) {
                return Err(0xc0);
            }
            return Ok(unsafe { ptr::read_volatile(addr as *const u32) });
        }
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
            while cycle_count().wrapping_sub(start) < word(item, 12) * self.ticks {}
        }
        Ok(0)
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
