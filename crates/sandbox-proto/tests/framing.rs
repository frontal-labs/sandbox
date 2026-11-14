//! Integration tests for length-prefixed frame encoding/decoding.

use sandbox_proto::framing::{decode, encode, MAX_FRAME_BYTES};
use sandbox_types::ErrorCode;

#[test]
fn frames_round_trip_and_partial_input_waits() {
    let frame = encode(b"hello").unwrap();
    assert_eq!(decode(&frame[..3]).unwrap(), None);
    let (payload, used) = decode(&frame).unwrap().unwrap();
    assert_eq!(payload, b"hello");
    assert_eq!(used, frame.len());
}

#[test]
fn oversize_length_is_refused_before_reading_the_body() {
    // A header claiming 17 MiB with no body must fail, not wait for data.
    let header = ((MAX_FRAME_BYTES as u32) + 1).to_be_bytes();
    let err = decode(&header).unwrap_err();
    assert_eq!(err.code, ErrorCode::ResourceExhausted);
}

#[test]
fn oversize_payload_is_refused_on_encode() {
    let big = vec![0u8; MAX_FRAME_BYTES + 1];
    assert_eq!(encode(&big).unwrap_err().code, ErrorCode::ResourceExhausted);
}
