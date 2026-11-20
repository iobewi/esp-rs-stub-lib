use stub_core::rx::{DoubleBuffer, Poll, SlotState};
use stub_core::usb_rx::{RxFailure, RxPump, RxPumpError, RxPumpResult, RxRead, RxSource, RX_BURST};

struct SimRx {
    bytes: Vec<u8>,
    pos: usize,
    max_accept: usize,
    calls: usize,
    override_result: Option<Result<RxRead, RxFailure>>,
    consumed_on_override: usize,
}

impl SimRx {
    fn new(bytes: &[u8], max_accept: usize) -> Self {
        Self {
            bytes: bytes.to_vec(),
            pos: 0,
            max_accept,
            calls: 0,
            override_result: None,
            consumed_on_override: 0,
        }
    }
}

impl RxSource for SimRx {
    fn read(&mut self, buffer: &mut [u8]) -> Result<RxRead, RxFailure> {
        self.calls += 1;
        if let Some(result) = self.override_result {
            self.pos += self.consumed_on_override.min(self.bytes.len() - self.pos);
            return result;
        }
        let count = self
            .max_accept
            .min(buffer.len())
            .min(self.bytes.len() - self.pos);
        buffer[..count].copy_from_slice(&self.bytes[self.pos..self.pos + count]);
        self.pos += count;
        if count == 0 {
            if self.pos == self.bytes.len() {
                Ok(RxRead::Empty)
            } else {
                Ok(RxRead::NoProgress)
            }
        } else {
            Ok(RxRead::Data(count))
        }
    }
}

#[test]
fn rx_burst_is_bounded_and_slip_frame_survives_multiple_reads() {
    let mut payload = vec![0xC0];
    payload.extend((0..100).map(|i| (i % 100 + 1) as u8));
    payload.push(0xC0);
    let mut sim = SimRx::new(&payload, usize::MAX);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<128>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 64 })
    );
    assert_eq!(rx.poll(), Poll::Idle);
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 38 })
    );
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 100 });
    assert_eq!(rx.frame(0).unwrap().len(), 100);
    assert_eq!(sim.calls, 2);
    assert_eq!(pump.received_bytes, 102);
    assert_eq!(RX_BURST, 64);
}

#[test]
fn empty_read_is_single_call_and_does_not_spin() {
    let mut sim = SimRx::new(&[], 64);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(pump.pump_once(&mut sim, &mut rx), Ok(RxPumpResult::Empty));
    assert_eq!(sim.calls, 1);
    assert_eq!(pump.calls, 1);
}

#[test]
fn indeterminate_read_poison_prevents_further_reads_and_replay() {
    let mut sim = SimRx::new(&[0xC0, 0x42, 0xC0], 64);
    sim.override_result = Some(Err(RxFailure::Indeterminate(-7)));
    sim.consumed_on_override = 2;
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Err(RxPumpError::Source(RxFailure::Indeterminate(-7)))
    );
    assert!(pump.requires_resync());
    assert_eq!(sim.pos, 2);
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Err(RxPumpError::ResyncRequired)
    );
    assert_eq!(sim.calls, 1);
    // An external transport reset must first discard residual FIFO bytes.
    sim.pos = sim.bytes.len();
    sim.override_result = None;
    rx.reset_after_resync();
    pump.reset_after_resync();
    assert_eq!(pump.pump_once(&mut sim, &mut rx), Ok(RxPumpResult::Empty));
    assert_eq!(rx.poll(), Poll::Idle);
}

#[test]
fn impossible_read_count_is_poison_not_slice_overflow() {
    let mut sim = SimRx::new(&[0xC0, 0x12, 0xC0], 64);
    sim.override_result = Some(Ok(RxRead::Data(RX_BURST + 1)));
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Err(RxPumpError::Source(RxFailure::InvalidCount {
            offered: RX_BURST,
            returned: RX_BURST + 1
        }))
    );
    assert!(pump.requires_resync());
    assert_eq!(rx.slot_state(0), Some(SlotState::Empty));
    assert_eq!(sim.calls, 1);
}

#[test]
fn short_reads_preserve_escape_sequences() {
    let mut sim = SimRx::new(&[0xC0, 0xDB, 0xDC, 0xC0], 1);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    for _ in 0..4 {
        assert_eq!(
            pump.pump_once(&mut sim, &mut rx),
            Ok(RxPumpResult::Progress { received: 1 })
        );
    }
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.frame(0), Some(&[0xC0][..]));
}

#[test]
fn two_slip_frames_in_one_rom_read_survive_without_reordering() {
    let mut sim = SimRx::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0], 64);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();

    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 6 })
    );
    assert_eq!(sim.calls, 1);
    assert_eq!(pump.pending(), 3);
    assert_eq!(rx.frame(0), Some(&[0x11][..]));

    // Cannot poll the ROM before a slot has been rearmed, even with a
    // complete frame and pending software bytes.
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Paused { pending: 3 })
    );
    assert_eq!(sim.calls, 1);

    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert!(rx.is_armed());
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 0 })
    );
    assert_eq!(pump.pending(), 0);
    assert_eq!(sim.calls, 1);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.frame(0), Some(&[0x11][..]));
    rx.release_selected();
    assert_eq!(rx.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(rx.frame(1), Some(&[0x22][..]));
    assert_eq!(pump.received_bytes, 6);
}

#[test]
fn disarmed_rx_never_polls_rom_even_if_software_batch_is_empty() {
    let mut sim = SimRx::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0], 3);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 3 })
    );
    assert!(!rx.is_armed());
    assert_eq!(pump.pending(), 0);
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Paused { pending: 0 })
    );
    assert_eq!(sim.calls, 1);
    assert_eq!(sim.pos, 3);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 3 })
    );
    assert_eq!(sim.calls, 2);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    rx.release_selected();
    assert_eq!(rx.poll(), Poll::Ready { index: 1, len: 1 });
}

#[test]
fn resync_discards_pending_bytes_and_cannot_replay_them() {
    let mut sim = SimRx::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0], 64);
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 6 })
    );
    assert_eq!(pump.pending(), 3);
    // External reset discards pending bytes in the ROM and in the pump.
    rx.reset_after_resync();
    pump.reset_after_resync();
    assert_eq!(pump.pending(), 0);
    assert_eq!(pump.pump_once(&mut sim, &mut rx), Ok(RxPumpResult::Empty));
    assert_eq!(rx.poll(), Poll::Idle);
}

#[test]
fn positive_fifo_occupancy_with_zero_read_is_not_empty() {
    let mut sim = SimRx::new(&[0xC0, 0x45, 0xC0], 64);
    sim.override_result = Some(Ok(RxRead::NoProgress));
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::NoProgress)
    );
    assert_eq!(sim.pos, 0);
    assert_eq!(pump.pending(), 0);
    sim.override_result = None;
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Ok(RxPumpResult::Progress { received: 3 })
    );
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.frame(0), Some(&[0x45][..]));
}

#[test]
fn zero_data_count_is_rejected_not_mistaken_for_empty() {
    let mut sim = SimRx::new(&[0xC0], 64);
    sim.override_result = Some(Ok(RxRead::Data(0)));
    let mut pump = RxPump::new();
    let mut rx = DoubleBuffer::<8>::new();
    assert_eq!(
        pump.pump_once(&mut sim, &mut rx),
        Err(RxPumpError::Source(RxFailure::InvalidCount {
            offered: RX_BURST,
            returned: 0
        }))
    );
    assert!(pump.requires_resync());
}
