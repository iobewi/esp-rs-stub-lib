use stub_core::probe::{encode_slip, reply, ProbeError, MAX_ECHO, PROTOCOL_ID};
use stub_core::{SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_ESC_ESC};

#[test]
fn existing_ping_remains_byte_compatible() {
    let mut payload = [0; 64];
    let n = reply(b"PING", &mut payload).unwrap();
    assert_eq!(&payload[..n], b"PONG");
    let mut encoded = [0; 64];
    let m = encode_slip(&payload[..n], &mut encoded).unwrap();
    assert_eq!(&encoded[..m], b"\xC0PONG\xC0");
}

#[test]
fn reports_fixed_version_without_hardware_claims() {
    let mut out = [0; 64];
    let n = reply(b"INFO", &mut out).unwrap();
    assert_eq!(&out[..n], PROTOCOL_ID);
}

#[test]
fn echo_binary_and_slip_metacharacters() {
    let request = [b'E', b'C', b'H', b'O', 0, SLIP_END, SLIP_ESC, 255];
    let mut out = [0; 64];
    let n = reply(&request, &mut out).unwrap();
    assert_eq!(&out[..n], &request);
    let mut encoded = [0; 64];
    let m = encode_slip(&out[..n], &mut encoded).unwrap();
    assert_eq!(&encoded[..m],
        &[SLIP_END, b'E', b'C', b'H', b'O', 0, SLIP_ESC, SLIP_ESC_END,
          SLIP_ESC, SLIP_ESC_ESC, 255, SLIP_END]);
}

#[test]
fn full_echo_limit_is_accepted_and_plus_one_refused() {
    let mut req = [0u8; 4 + MAX_ECHO + 1];
    req[..4].copy_from_slice(b"ECHO");
    req[4..].fill(0x5a);
    let mut out = [0; 64];
    assert_eq!(reply(&req[..4 + MAX_ECHO], &mut out), Ok(4 + MAX_ECHO));
    assert_eq!(reply(&req, &mut out), Err(ProbeError::PayloadTooLong));
}

#[test]
fn unsupported_and_short_output_leave_memory_untouched() {
    let mut out = [0xA5; 4];
    assert_eq!(reply(b"NOOP", &mut out), Err(ProbeError::Unsupported));
    assert_eq!(reply(b"INFO", &mut out), Err(ProbeError::OutputTooSmall));
    assert_eq!(out, [0xA5; 4]);
    assert_eq!(reply(b"PING", &mut out), Ok(4));
    assert_eq!(out, *b"PONG");
}

#[test]
fn encoded_size_is_checked_atomically() {
    let mut out = [0xA5; 5];
    assert_eq!(encode_slip(&[SLIP_END, SLIP_ESC], &mut out), Err(ProbeError::OutputTooSmall));
    assert_eq!(out, [0xA5; 5]);
    let mut good = [0; 6];
    assert_eq!(encode_slip(&[SLIP_END, SLIP_ESC], &mut good), Ok(6));
    assert_eq!(good, [SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_ESC, SLIP_ESC_ESC, SLIP_END]);
}

#[test]
fn empty_payload_encodes_to_two_delimiters() {
    let mut out = [0; 2];
    assert_eq!(encode_slip(b"", &mut out), Ok(2));
    assert_eq!(out, [SLIP_END, SLIP_END]);
}
