use stub_core::tx::{
    FifoFailure, FifoFill, PumpResult, TxConfigError, TxError, TxQueue, TxState,
    USB_TX_STAGING_BYTES,
};

#[derive(Default)]
struct MockFifo {
    accepted: Vec<u8>,
    limit: usize,
    calls: usize,
    override_return: Option<Result<usize, FifoFailure>>,
    reported_offers: Vec<usize>,
    consumed_on_override: usize,
}

impl FifoFill for MockFifo {
    fn fill(&mut self, bytes: &[u8]) -> Result<usize, FifoFailure> {
        self.calls += 1;
        self.reported_offers.push(bytes.len());
        if let Some(ret) = self.override_return {
            let n = core::cmp::min(bytes.len(), self.consumed_on_override);
            self.accepted.extend_from_slice(&bytes[..n]);
            return ret;
        }
        let n = core::cmp::min(bytes.len(), self.limit);
        self.accepted.extend_from_slice(&bytes[..n]);
        Ok(n)
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
fn invalid_return_blocks_replay_even_if_fifo_consumed_bytes() {
    let mut q = TxQueue::<8>::new();
    assert_eq!(q.enqueue(&[9, 8, 7]), 3);
    // Faulty adapter consumes two bytes but claims it accepted four.
    let mut fifo = MockFifo {
        override_return: Some(Ok(4)),
        consumed_on_override: 2,
        ..MockFifo::default()
    };
    assert_eq!(
        q.pump_once(&mut fifo),
        Err(TxError::InvalidAcceptance {
            offered: 3,
            returned: 4
        })
    );
    assert_eq!(fifo.accepted, [9, 8]);
    assert_eq!(q.state(), TxState::ResyncRequired);
    assert_eq!(q.pending(), 3);
    assert_eq!(q.enqueue(&[6]), 0);
    fifo.override_return = None;
    fifo.limit = 8;
    // No duplicate transmission is possible while poisoned.
    assert_eq!(q.pump_once(&mut fifo), Err(TxError::ResyncRequired));
    assert_eq!(fifo.calls, 1);
    assert_eq!(fifo.accepted, [9, 8]);
    // A new session/SLIP frame must be established outside the queue first.
    q.reset_after_resync();
    assert_eq!(q.pending(), 0);
    assert_eq!(q.state(), TxState::Ready);
    assert_eq!(q.enqueue(&[0x42]), 1);
    assert_eq!(
        q.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 1,
            remaining: 0
        })
    );
    assert_eq!(fifo.accepted, [9, 8, 0x42]);
}

#[test]
fn indeterminate_negative_return_blocks_replay() {
    let mut q = TxQueue::<8>::new();
    assert_eq!(q.enqueue(&[1, 2, 3]), 3);
    let mut fifo = MockFifo {
        override_return: Some(Err(FifoFailure::Indeterminate(-5))),
        consumed_on_override: 1,
        ..MockFifo::default()
    };
    assert_eq!(q.pump_once(&mut fifo), Err(TxError::Indeterminate(-5)));
    assert_eq!(q.state(), TxState::ResyncRequired);
    assert_eq!(fifo.accepted, [1]);
    assert_eq!(q.pump_once(&mut fifo), Err(TxError::ResyncRequired));
    assert_eq!(fifo.calls, 1);
}

#[test]
fn proven_no_progress_error_allows_retries() {
    let mut q = TxQueue::<8>::new();
    assert_eq!(q.enqueue(&[9, 8, 7]), 3);
    let mut fifo = MockFifo {
        override_return: Some(Err(FifoFailure::NoProgress(-11))),
        ..MockFifo::default()
    };
    assert_eq!(q.pump_once(&mut fifo), Err(TxError::NoProgress(-11)));
    assert_eq!(q.state(), TxState::Ready);
    assert_eq!(q.pending(), 3);
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
fn max_offer_is_independent_from_software_capacity() {
    let payload: Vec<u8> = (0..=255).cycle().take(4096).collect();
    for capacity in [64, 4096] {
        // Different queue capacities, identical maximum ROM offer length.
        let mut fifo = MockFifo {
            limit: 64,
            ..MockFifo::default()
        };
        if capacity == 64 {
            let mut q = TxQueue::<64>::with_max_offer(64).unwrap();
            for chunk in payload.chunks(64) {
                assert_eq!(q.enqueue(chunk), chunk.len());
                while q.pending() > 0 {
                    q.pump_once(&mut fifo).unwrap();
                }
            }
        } else {
            let mut q = TxQueue::<4096>::with_max_offer(64).unwrap();
            assert_eq!(q.enqueue(&payload), payload.len());
            while q.pending() > 0 {
                q.pump_once(&mut fifo).unwrap();
            }
        }
        assert_eq!(fifo.accepted, payload);
        assert!(fifo.reported_offers.iter().all(|&offered| offered <= 64));
        assert_eq!(fifo.calls, 64);
    }
    let mut large = TxQueue::<4096>::with_max_offer(256).unwrap();
    let mut fifo = MockFifo {
        limit: 256,
        ..MockFifo::default()
    };
    assert_eq!(large.enqueue(&payload), 4096);
    assert_eq!(
        large.pump_once(&mut fifo),
        Ok(PumpResult::Progress {
            accepted: 256,
            remaining: 3840
        })
    );
    assert_eq!(fifo.reported_offers, [256]);
}

#[test]
fn zero_or_unrepresentable_offer_limit_is_rejected() {
    assert!(matches!(
        TxQueue::<4096>::with_max_offer(0),
        Err(TxConfigError::InvalidMaxOffer)
    ));
    assert!(matches!(
        TxQueue::<4096>::with_max_offer(i32::MAX as usize + 1),
        Err(TxConfigError::InvalidMaxOffer)
    ));
    assert_eq!(TxQueue::<4096>::new().max_offer(), 64);
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
