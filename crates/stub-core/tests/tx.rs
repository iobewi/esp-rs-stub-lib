use stub_core::tx::{FifoFill, PumpResult, TxError, TxQueue, USB_TX_STAGING_BYTES};

#[derive(Default)]
struct MockFifo {
    accepted: Vec<u8>,
    limit: usize,
    calls: usize,
    override_return: Option<i32>,
}

impl FifoFill for MockFifo {
    fn fill(&mut self, bytes: &[u8]) -> i32 {
        self.calls += 1;
        if let Some(ret) = self.override_return {
            return ret;
        }
        let n = core::cmp::min(bytes.len(), self.limit);
        self.accepted.extend_from_slice(&bytes[..n]);
        n as i32
    }
}

#[test]
fn partial_accepts_keep_every_byte_in_order() {
    for limit in [64, 32, 1] {
        let payload: Vec<u8> = (0..=255).cycle().take(4096).collect();
        let mut q = TxQueue::<4096>::new();
        let mut fifo = MockFifo {
            limit,
            ..MockFifo::default()
        };
        assert_eq!(q.enqueue(&payload), payload.len());
        while q.pending() > 0 {
            assert!(matches!(
                q.pump_once(&mut fifo),
                Ok(PumpResult::Progress { .. })
            ));
        }
        assert_eq!(fifo.accepted, payload, "acceptance limit: {limit}");
        assert_eq!(q.counters().accepted_bytes, payload.len() as u64);
        assert_eq!(q.counters().fill_calls as usize, payload.len() / limit);
    }
}

#[test]
fn zero_accept_is_explicit_backpressure_without_drop_or_spin() {
    let mut q = TxQueue::<8>::new();
    let mut fifo = MockFifo::default();
    assert_eq!(q.enqueue(&[1, 2, 3]), 3);
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Backpressure { remaining: 3 })
    );
    assert_eq!(fifo.calls, 1);
    assert_eq!(q.pending(), 3);
    fifo.limit = 8;
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 3,
            remaining: 0
        })
    );
    assert_eq!(fifo.accepted, [1, 2, 3]);
    assert_eq!(q.counters().zero_accepts, 1);
}

#[test]
fn enqueue_reports_backpressure_and_compacts_without_reordering() {
    let mut q = TxQueue::<4>::new();
    let mut fifo = MockFifo {
        limit: 2,
        ..MockFifo::default()
    };
    assert_eq!(q.enqueue(&[1, 2, 3, 4, 5]), 4);
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 2,
            remaining: 2
        })
    );
    assert_eq!(q.enqueue(&[5, 6, 7]), 2);
    fifo.limit = 4;
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 4,
            remaining: 0
        })
    );
    assert_eq!(fifo.accepted, [1, 2, 3, 4, 5, 6]);
    assert_eq!(q.enqueue(&[7]), 1);
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 1,
            remaining: 0
        })
    );
    assert_eq!(fifo.accepted, [1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn invalid_return_does_not_release_pending_bytes() {
    let mut q = TxQueue::<8>::new();
    assert_eq!(q.enqueue(&[9, 8, 7]), 3);
    let mut fifo = MockFifo {
        override_return: Some(4),
        ..MockFifo::default()
    };
    assert_eq!(
        q.pump_once(&mut fifo),
        Err(TxError::InvalidAcceptance {
            offered: 3,
            returned: 4
        })
    );
    fifo.override_return = Some(-5);
    assert_eq!(q.pump_once(&mut fifo), Err(TxError::RomFailure(-5)));
    assert_eq!(q.pending(), 3);
    assert_eq!(q.counters().invalid_accepts, 2);
    fifo.override_return = None;
    fifo.limit = 3;
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 3,
            remaining: 0
        })
    );
    assert_eq!(fifo.accepted, [9, 8, 7]);
}

#[test]
fn empty_pump_does_not_call_rom() {
    let mut q = TxQueue::<1>::new();
    let mut fifo = MockFifo {
        limit: 1,
        ..MockFifo::default()
    };
    assert_eq!(q.pump_once(&mut fifo), Ok(PumpResult::Empty));
    assert_eq!(fifo.calls, 0);
    assert_eq!(q.counters().fill_calls, 0);
}

#[test]
fn staging_size_is_an_experiment_not_fifo_capacity() {
    assert_eq!(USB_TX_STAGING_BYTES, 4096);
    let mut q = TxQueue::<USB_TX_STAGING_BYTES>::new();
    assert_eq!(q.enqueue(&[0xaa; 64]), 64);
    assert_eq!(q.capacity(), 4096);
}
