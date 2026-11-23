use stub_core::rx::{DoubleBuffer, Poll, SlotState, FRAME_BUFFER_SIZE};
use stub_core::slip::{send_frame, DecodeEvent, DecodeState, SlipDecoder, TxSink};
use stub_core::{SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_ESC_ESC};

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    flushes: usize,
    fail_after: Option<usize>,
}

impl TxSink for Sink {
    type Error = &'static str;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.fail_after == Some(self.bytes.len()) {
            return Err("write failed");
        }
        self.bytes.push(byte);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.flushes += 1;
        Ok(())
    }
}

fn feed<const N: usize>(rx: &mut DoubleBuffer<N>, values: &[u8]) {
    for &byte in values {
        rx.receive(byte);
    }
}

#[test]
fn encode_escaped_bytes_and_flush() {
    let mut sink = Sink::default();
    send_frame(&[0x11, SLIP_END, SLIP_ESC, 0x22], &mut sink).unwrap();
    assert_eq!(
        sink.bytes,
        [
            SLIP_END,
            0x11,
            SLIP_ESC,
            SLIP_ESC_END,
            SLIP_ESC,
            SLIP_ESC_ESC,
            0x22,
            SLIP_END
        ]
    );
    assert_eq!(sink.flushes, 1);
}

#[test]
fn encode_empty_payload() {
    let mut sink = Sink::default();
    send_frame(&[], &mut sink).unwrap();
    assert_eq!(sink.bytes, [SLIP_END, SLIP_END]);
    assert_eq!(sink.flushes, 1);
}

#[test]
fn send_failure_propagates_without_flush() {
    let mut sink = Sink {
        fail_after: Some(2),
        ..Sink::default()
    };
    assert_eq!(send_frame(&[SLIP_ESC], &mut sink), Err("write failed"));
    assert_eq!(sink.flushes, 0);
}

#[test]
fn decode_waits_for_delimiter_and_ignores_empty_gap() {
    let mut decoder = SlipDecoder::new();
    assert_eq!(decoder.push(0x12, true), DecodeEvent::None);
    assert_eq!(decoder.push(SLIP_END, true), DecodeEvent::None);
    assert_eq!(decoder.push(SLIP_END, true), DecodeEvent::None);
    assert_eq!(decoder.state(), DecodeState::NoFrame);
    assert_eq!(decoder.push(0x23, true), DecodeEvent::None);
    assert_eq!(decoder.push(SLIP_END, true), DecodeEvent::None);
    assert_eq!(decoder.state(), DecodeState::InFrame);
    assert_eq!(decoder.push(0x42, true), DecodeEvent::Byte(0x42));
    assert_eq!(decoder.push(SLIP_END, true), DecodeEvent::Complete);
    assert_eq!(decoder.state(), DecodeState::NoFrame);
}

#[test]
fn empty_frame_followed_by_out_of_frame_data_is_ignored() {
    let mut rx = DoubleBuffer::<4>::new();
    // C0 C0 23 C0: no completed frame, final C0 opens the next one.
    feed(&mut rx, &[SLIP_END, SLIP_END, 0x23, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Idle);
    // 42 C0 completes the frame started by the prior C0.
    feed(&mut rx, &[0x42, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.frame(0), Some(&[0x42][..]));
}

#[test]
fn rx_error_can_supersede_selected_frame_without_losing_it() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, 0x11, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    // A newly received bad escape takes priority at the next poll.
    feed(&mut rx, &[SLIP_END, SLIP_ESC, 0xaa]);
    assert_eq!(rx.poll(), Poll::Error { index: 1 });
    rx.release_selected();
    assert_eq!(rx.slot_state(1), Some(SlotState::Empty));
    assert_eq!(rx.frame(0), Some(&[0x11][..]));
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    rx.release_selected();
    assert_eq!(rx.slot_state(0), Some(SlotState::Empty));
}

#[test]
fn decode_rejects_bad_escape_and_recovers() {
    let mut decoder = SlipDecoder::new();
    decoder.push(SLIP_END, true);
    decoder.push(SLIP_ESC, true);
    assert_eq!(decoder.push(0xaa, true), DecodeEvent::InvalidEscape);
    assert_eq!(decoder.state(), DecodeState::NoFrame);
    decoder.push(SLIP_END, true);
    assert_eq!(decoder.push(3, true), DecodeEvent::Byte(3));
    assert_eq!(decoder.push(SLIP_END, true), DecodeEvent::Complete);
}

#[test]
fn one_frame_round_trip() {
    let mut rx = DoubleBuffer::<8>::new();
    feed(
        &mut rx,
        &[
            SLIP_END,
            1,
            SLIP_ESC,
            SLIP_ESC_END,
            SLIP_ESC,
            SLIP_ESC_ESC,
            SLIP_END,
        ],
    );
    let result = rx.poll();
    let Poll::Ready { index, len } = result else {
        panic!("not ready");
    };
    assert_eq!(len, 3);
    assert_eq!(rx.frame(index), Some(&[1, SLIP_END, SLIP_ESC][..]));
    rx.release_selected();
    rx.release_selected();
    assert_eq!(rx.slot_state(index), Some(SlotState::Empty));
}

#[test]
fn exact_capacity_is_accepted() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, 1, 2, 3, 4, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 4 });
    assert_eq!(rx.frame(0), Some(&[1, 2, 3, 4][..]));
}

#[test]
fn overflow_is_error_and_never_truncates_to_valid_frame() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, 1, 2, 3, 4, 5, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Error { index: 0 });
    assert!(rx.frame(0).is_none());
    rx.release_selected();
    feed(&mut rx, &[SLIP_END, 7, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 1, len: 1 });
}

#[test]
fn back_to_back_frames_use_both_buffers() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, 1, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    assert_eq!(rx.rx_index(), 1);
    feed(&mut rx, &[SLIP_END, 2, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
    rx.release_selected();
    assert_eq!(rx.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(rx.frame(1), Some(&[2][..]));
    rx.release_selected();
    assert_eq!(rx.poll(), Poll::Idle);
    assert!(rx.is_armed());
}

#[test]
fn both_slots_busy_discard_unarmed_input() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, 1, SLIP_END]);
    rx.poll();
    feed(&mut rx, &[SLIP_END, 2, SLIP_END]);
    rx.poll();
    assert!(!rx.is_armed());
    feed(&mut rx, &[SLIP_END, 99, SLIP_END]);
    assert_eq!(rx.frame(1), Some(&[2][..]));
    rx.release_selected();
    // The pending slot 1 is returned; freed slot 0 becomes the RX slot.
    assert_eq!(rx.poll(), Poll::Ready { index: 1, len: 1 });
    assert_eq!(rx.rx_index(), 0);
    assert!(rx.is_armed());
    rx.release_selected();
    feed(&mut rx, &[SLIP_END, 3, SLIP_END]);
    assert_eq!(rx.poll(), Poll::Ready { index: 0, len: 1 });
}

#[test]
fn invalid_escape_marks_slot_error() {
    let mut rx = DoubleBuffer::<4>::new();
    feed(&mut rx, &[SLIP_END, SLIP_ESC, 0xaa]);
    assert_eq!(rx.poll(), Poll::Error { index: 0 });
    rx.release_selected();
    assert_eq!(rx.poll(), Poll::Idle);
}

#[test]
fn buffer_size_matches_upstream() {
    assert_eq!(FRAME_BUFFER_SIZE, 16_647);
}

#[test]
fn full_sized_frame_boundary() {
    let mut rx = Box::new(DoubleBuffer::<FRAME_BUFFER_SIZE>::new());
    rx.receive(SLIP_END);
    for _ in 0..FRAME_BUFFER_SIZE {
        rx.receive(0x5a);
    }
    rx.receive(SLIP_END);
    assert_eq!(
        rx.poll(),
        Poll::Ready {
            index: 0,
            len: FRAME_BUFFER_SIZE
        }
    );
    assert_eq!(rx.frame(0).unwrap()[FRAME_BUFFER_SIZE - 1], 0x5a);
}

#[test]
fn in_place_receiver_arms_both_slots_like_the_default_receiver() {
    let mut storage = core::mem::MaybeUninit::uninit();
    let frames = DoubleBuffer::<8>::init_in(&mut storage);
    for byte in [0xc0, 0x11, 0xc0] {
        frames.receive(byte);
    }
    let Poll::Ready { index: first, .. } = frames.poll() else {
        panic!("first frame missing")
    };
    assert_eq!(frames.frame(first), Some(&[0x11][..]));
    for byte in [0xc0, 0x22, 0xc0] {
        frames.receive(byte);
    }
    frames.release_selected();
    let Poll::Ready { index: second, .. } = frames.poll() else {
        panic!("second frame missing")
    };
    assert_ne!(first, second);
    assert_eq!(frames.frame(second), Some(&[0x22][..]));
}
