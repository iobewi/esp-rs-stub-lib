use stub_core::rx::{DoubleBuffer, Poll};
use stub_core::usb_irq::{
    LineDecision, RxIrqCoordinator, RxIrqMask, RxIrqSignal, RxLineState, ServiceError,
    ServiceResult,
};
use stub_core::usb_rx::{RxFailure, RxPump, RxRead, RxSource};

struct FakeMask {
    masked: bool,
    masks: usize,
    unmasks: usize,
    cdc_quiesced: bool,
    line_decision: LineDecision,
}

impl Default for FakeMask {
    fn default() -> Self {
        Self {
            masked: false,
            masks: 0,
            unmasks: 0,
            cdc_quiesced: false,
            line_decision: LineDecision::Benign,
        }
    }
}

impl RxLineState for FakeMask {
    fn quiesce_cdc_callbacks(&mut self) {
        assert!(self.masked);
        self.cdc_quiesced = true;
    }

    fn inspect_line_state_quiesced(&mut self) -> LineDecision {
        assert!(self.cdc_quiesced);
        self.line_decision
    }
}

impl RxIrqMask for FakeMask {
    fn mask_rx(&mut self) {
        self.masked = true;
        self.masks += 1;
    }

    fn unmask_rx(&mut self) {
        self.masked = false;
        self.unmasks += 1;
    }
}

struct Fifo {
    bytes: Vec<u8>,
    cursor: usize,
    calls: usize,
    failed: bool,
}

impl Fifo {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            cursor: 0,
            calls: 0,
            failed: false,
        }
    }
}

impl RxSource for Fifo {
    fn read(&mut self, into: &mut [u8]) -> Result<RxRead, RxFailure> {
        self.calls += 1;
        if self.failed {
            return Err(RxFailure::Indeterminate(-1));
        }
        let n = into.len().min(self.bytes.len() - self.cursor);
        into[..n].copy_from_slice(&self.bytes[self.cursor..self.cursor + n]);
        self.cursor += n;
        if n == 0 {
            Ok(RxRead::Empty)
        } else {
            Ok(RxRead::Data(n))
        }
    }
}

fn step<const N: usize>(
    coordinator: &mut RxIrqCoordinator,
    signal: &RxIrqSignal,
    hw: &mut FakeMask,
    fifo: &mut Fifo,
    pump: &mut RxPump,
    frames: &mut DoubleBuffer<N>,
) -> Result<ServiceResult, ServiceError> {
    coordinator.service_once(signal, hw, fifo, pump, frames)
}

#[test]
fn irq_never_touches_rx_buffers_and_rearms_only_after_drain() {
    let mut irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0]);

    // The callback models masking RX before publishing a notification.
    hw.mask_rx();
    irq.notify_rx_masked();
    assert_eq!(fifo.calls, 0);
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Progress { received: 6 })
    );
    assert_eq!(pump.pending(), 3);
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 0);
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Paused { pending: 3 })
    );
    assert_eq!(fifo.calls, 1);
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Progress { received: 0 })
    );
    assert_eq!(pump.pending(), 0);
    assert_eq!(fifo.calls, 1);
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    frames.release_selected();
    assert_eq!(frames.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(frames.frame(1), Some(&[0x22][..]));
    // A free slot is already rearmed by poll(), but interrupts remain
    // masked until the current ROM FIFO is confirmed empty.
    assert!(frames.is_armed());
    assert_eq!(hw.unmasks, 0);
    frames.release_selected();
    assert_eq!(frames.poll(), Poll::Idle);
    assert!(frames.is_armed());
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Drained)
    );
    assert!(!hw.masked);
    assert_eq!(hw.unmasks, 1);
    assert_eq!(fifo.calls, 2);
    irq.reset_after_quiesce();
}

#[test]
fn no_notification_means_no_fifo_read() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Idle)
    );
    assert_eq!(fifo.calls, 0);
    // The first idle foreground pass arms RX; otherwise no first IRQ
    // could ever arrive after startup with hardware initially masked.
    assert_eq!(hw.unmasks, 1);
    assert!(!hw.masked);
}

#[test]
fn a_link_change_blocks_fifo_and_unmask_until_external_recovery() {
    let mut irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    irq.notify_line_state_changed();
    irq.notify_rx_masked();
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::LinkChange)
    );
    assert_eq!(fifo.calls, 0);
    assert_eq!(hw.unmasks, 0);
    coordinator.reset_after_resync(&mut irq, &mut hw, &mut pump, &mut frames);
    assert!(!irq.link_change_pending());
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Idle)
    );
    assert_eq!(fifo.calls, 0);
}

#[test]
fn an_ambiguous_rom_read_latches_failure_without_retry() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    fifo.failed = true;
    irq.notify_rx_masked();
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Err(ServiceError::Rx(stub_core::usb_rx::RxPumpError::Source(
            RxFailure::Indeterminate(-1)
        )))
    );
    assert!(coordinator.requires_resync());
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Err(ServiceError::ResyncRequired)
    );
    assert_eq!(fifo.calls, 1);
    assert_eq!(hw.unmasks, 0);
}

#[test]
fn repeated_notifications_coalesce_without_dropping_new_wakeup() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[]);
    irq.notify_rx_masked();
    irq.notify_rx_masked();
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Drained)
    );
    assert_eq!(fifo.calls, 1);
    irq.notify_rx_masked();
    assert_eq!(
        step(
            &mut coordinator,
            &irq,
            &mut hw,
            &mut fifo,
            &mut pump,
            &mut frames
        ),
        Ok(ServiceResult::Drained)
    );
    assert_eq!(fifo.calls, 2);
}

struct WakeOnUnmask<'a> {
    signal: &'a RxIrqSignal,
    masked: bool,
    notify_once: bool,
    unmasks: usize,
}

impl RxIrqMask for WakeOnUnmask<'_> {
    fn mask_rx(&mut self) {
        self.masked = true;
    }

    fn unmask_rx(&mut self) {
        self.unmasks += 1;
        self.masked = false;
        if self.notify_once {
            // Simulate the hardware immediately raising RX on unmask:
            // the callback masks RX, then publishes its notification.
            self.masked = true;
            self.signal.notify_rx_masked();
            self.notify_once = false;
        }
    }
}

#[test]
fn interrupt_racing_with_first_unmask_is_not_lost() {
    let irq = RxIrqSignal::new();
    let mut hw = WakeOnUnmask {
        signal: &irq,
        masked: true,
        notify_once: true,
        unmasks: 0,
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x42, 0xC0]);

    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Idle)
    );
    assert_eq!(hw.unmasks, 1);
    assert!(hw.masked);
    assert!(coordinator.is_masked());
    assert_eq!(fifo.calls, 0);

    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 3 })
    );
    assert_eq!(fifo.calls, 1);
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(frames.frame(0), Some(&[0x42][..]));
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 1);
}

#[test]
fn zero_progress_does_not_unmask_and_data_can_resume_without_new_irq() {
    struct Stalled<'a> {
        bytes: &'a [u8],
        stall: bool,
        calls: usize,
    }
    impl RxSource for Stalled<'_> {
        fn read(&mut self, buf: &mut [u8]) -> Result<RxRead, RxFailure> {
            self.calls += 1;
            if self.stall {
                return Ok(RxRead::NoProgress);
            }
            if self.bytes.is_empty() {
                return Ok(RxRead::Empty);
            }
            let n = self.bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(RxRead::Data(n))
        }
    }
    let irq = RxIrqSignal::new();
    irq.notify_rx_masked();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut source = Stalled {
        bytes: &[0xC0, 0x55, 0xC0],
        stall: true,
        calls: 0,
    };
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut source, &mut pump, &mut frames),
        Ok(ServiceResult::NoProgress)
    );
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 0);
    assert_eq!(source.calls, 1);

    // No new interrupt. Foreground must continue polling the stalled FIFO.
    source.stall = false;
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut source, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 3 })
    );
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(frames.frame(0), Some(&[0x55][..]));
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 0);
    assert_eq!(source.calls, 2);

    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut source, &mut pump, &mut frames),
        Ok(ServiceResult::Drained)
    );
    assert!(!hw.masked);
    assert_eq!(hw.unmasks, 1);
}

#[test]
fn benign_line_change_ack_preserves_completed_frame_and_pending_rx() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0]);
    irq.notify_rx_masked();
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 6 })
    );
    assert_eq!(pump.pending(), 3);
    assert_eq!(frames.frame(0), Some(&[0x11][..]));
    irq.notify_line_state_changed();
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::LinkChange)
    );
    let reads = fifo.calls;
    // Inspection must take place after CDC callback quiescence.
    assert_eq!(
        coordinator.resolve_line_change(&irq, &mut hw),
        LineDecision::Benign
    );
    assert!(!irq.link_change_pending());
    assert_eq!(pump.pending(), 3);
    assert_eq!(frames.frame(0), Some(&[0x11][..]));
    assert_eq!(fifo.calls, reads);
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 0);

    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 0 })
    );
    assert_eq!(fifo.calls, reads);
    assert_eq!(frames.frame(0), Some(&[0x11][..]));
    frames.release_selected();
    assert_eq!(frames.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(frames.frame(1), Some(&[0x22][..]));
}

#[test]
fn benign_line_change_ack_keeps_undrained_fifo_work() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask {
        masked: true,
        ..Default::default()
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x77, 0xC0]);
    irq.notify_rx_masked();
    irq.notify_line_state_changed();
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::LinkChange)
    );
    assert_eq!(
        coordinator.resolve_line_change(&irq, &mut hw),
        LineDecision::Benign
    );
    assert_eq!(
        coordinator.service_once(&irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 3 })
    );
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(frames.frame(0), Some(&[0x77][..]));
}

struct RacingLineMask<'a> {
    signal: &'a RxIrqSignal,
    masked: bool,
    quiesced: bool,
    line: LineDecision,
    inject_on_quiesce: bool,
    inspections: usize,
}

impl RxIrqMask for RacingLineMask<'_> {
    fn mask_rx(&mut self) {
        self.masked = true;
    }

    fn unmask_rx(&mut self) {
        self.masked = false;
    }
}

impl RxLineState for RacingLineMask<'_> {
    fn quiesce_cdc_callbacks(&mut self) {
        assert!(self.masked);
        if self.inject_on_quiesce {
            // The second callback arrives just BEFORE full quiescence:
            // an RTS reset edge supersedes the previously benign state.
            self.signal.notify_line_state_changed();
            self.line = LineDecision::ResyncRequired;
            self.inject_on_quiesce = false;
        }
        self.quiesced = true;
    }

    fn inspect_line_state_quiesced(&mut self) -> LineDecision {
        assert!(self.quiesced, "line inspection must be under CDC quiescence");
        self.inspections += 1;
        self.line
    }
}

#[test]
fn second_rts_event_during_quiescence_cannot_be_acknowledged_as_benign() {
    let signal = RxIrqSignal::new();
    signal.notify_line_state_changed();
    let mut hw = RacingLineMask {
        signal: &signal,
        masked: false,
        quiesced: false,
        line: LineDecision::Benign,
        inject_on_quiesce: true,
        inspections: 0,
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x41, 0xC0]);

    assert_eq!(
        coordinator.service_once(&signal, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::LinkChange)
    );
    assert_eq!(fifo.calls, 0);

    // A stale pre-quiescence inspection would have said Benign.
    assert_eq!(
        coordinator.resolve_line_change(&signal, &mut hw),
        LineDecision::ResyncRequired
    );
    assert_eq!(hw.inspections, 1);
    assert!(hw.quiesced);
    assert!(hw.masked);
    assert!(signal.link_change_pending());
    assert_eq!(
        coordinator.service_once(&signal, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::LinkChange)
    );
    assert_eq!(fifo.calls, 0);
    assert_eq!(pump.pending(), 0);
}

#[test]
fn benign_inspection_under_quiescence_clears_line_flag_without_discarding_rx() {
    let signal = RxIrqSignal::new();
    let mut hw = RacingLineMask {
        signal: &signal,
        masked: false,
        quiesced: false,
        line: LineDecision::Benign,
        inject_on_quiesce: false,
        inspections: 0,
    };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0]);

    signal.notify_rx_masked();
    assert_eq!(
        coordinator.service_once(&signal, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 6 })
    );
    assert_eq!(pump.pending(), 3);
    signal.notify_line_state_changed();
    assert_eq!(
        coordinator.resolve_line_change(&signal, &mut hw),
        LineDecision::Benign
    );
    assert_eq!(hw.inspections, 1);
    assert!(hw.quiesced);
    assert!(!signal.link_change_pending());
    assert_eq!(pump.pending(), 3);
    assert_eq!(frames.frame(0), Some(&[0x11][..]));
}
