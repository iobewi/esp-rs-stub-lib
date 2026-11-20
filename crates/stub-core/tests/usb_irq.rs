use stub_core::rx::{DoubleBuffer, Poll};
use stub_core::usb_irq::{RxIrqCoordinator, RxIrqMask, RxIrqSignal, ServiceError, ServiceResult};
use stub_core::usb_rx::{RxFailure, RxPump, RxSource};

#[derive(Default)]
struct FakeMask {
    masked: bool,
    masks: usize,
    unmasks: usize,
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
    fn read(&mut self, into: &mut [u8]) -> Result<usize, RxFailure> {
        self.calls += 1;
        if self.failed {
            return Err(RxFailure::Indeterminate(-1));
        }
        let n = into.len().min(self.bytes.len() - self.cursor);
        into[..n].copy_from_slice(&self.bytes[self.cursor..self.cursor + n]);
        self.cursor += n;
        Ok(n)
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
    let mut hw = FakeMask { masked: true, ..Default::default() };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x11, 0xC0, 0xC0, 0x22, 0xC0]);

    // The callback models masking RX before publishing a notification.
    hw.mask_rx();
    irq.notify_rx_masked();
    assert_eq!(fifo.calls, 0);
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 6 }));
    assert_eq!(pump.pending(), 3);
    assert!(hw.masked);
    assert_eq!(hw.unmasks, 0);
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Paused { pending: 3 }));
    assert_eq!(fifo.calls, 1);
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Progress { received: 0 }));
    assert_eq!(pump.pending(), 0);
    assert_eq!(fifo.calls, 1);
    assert_eq!(frames.poll(), Poll::Ready { index: 0, len: 1 });
    frames.release_selected();
    assert_eq!(frames.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(frames.frame(1), Some(&[0x22][..]));
    assert!(!frames.is_armed());
    assert_eq!(hw.unmasks, 0);
    frames.release_selected();
    assert_eq!(frames.poll(), Poll::Idle);
    assert!(frames.is_armed());
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Drained));
    assert!(!hw.masked);
    assert_eq!(hw.unmasks, 1);
    assert_eq!(fifo.calls, 2);
    irq.reset_after_quiesce();
}

#[test]
fn no_notification_means_no_fifo_read() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask { masked: true, ..Default::default() };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Idle));
    assert_eq!(fifo.calls, 0);
}

#[test]
fn a_link_change_blocks_fifo_and_unmask_until_external_recovery() {
    let mut irq = RxIrqSignal::new();
    let mut hw = FakeMask { masked: true, ..Default::default() };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    irq.notify_line_state_changed();
    irq.notify_rx_masked();
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::LinkChange));
    assert_eq!(fifo.calls, 0);
    assert_eq!(hw.unmasks, 0);
    coordinator.reset_after_resync(&mut irq, &mut hw, &mut pump, &mut frames);
    assert!(!irq.link_change_pending());
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Idle));
    assert_eq!(fifo.calls, 0);
}

#[test]
fn an_ambiguous_rom_read_latches_failure_without_retry() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask { masked: true, ..Default::default() };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[0xC0, 0x33, 0xC0]);
    fifo.failed = true;
    irq.notify_rx_masked();
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Err(ServiceError::Rx(stub_core::usb_rx::RxPumpError::Source(RxFailure::Indeterminate(-1)))));
    assert!(coordinator.requires_resync());
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Err(ServiceError::ResyncRequired));
    assert_eq!(fifo.calls, 1);
    assert_eq!(hw.unmasks, 0);
}

#[test]
fn repeated_notifications_coalesce_without_dropping_new_wakeup() {
    let irq = RxIrqSignal::new();
    let mut hw = FakeMask { masked: true, ..Default::default() };
    let mut coordinator = RxIrqCoordinator::new();
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<8>::new();
    let mut fifo = Fifo::new(&[]);
    irq.notify_rx_masked();
    irq.notify_rx_masked();
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Drained));
    assert_eq!(fifo.calls, 1);
    irq.notify_rx_masked();
    assert_eq!(step(&mut coordinator, &irq, &mut hw, &mut fifo, &mut pump, &mut frames),
        Ok(ServiceResult::Drained));
    assert_eq!(fifo.calls, 2);
}
