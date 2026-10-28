#![no_main]

//! Fuzzes the host and guest wire decoders (PLAN.md §4.10, §5, §7): the length
//! prefixed frames and every generated guest message. Decoding untrusted bytes
//! must never panic, must never allocate past the frame cap, and a message that
//! decodes must survive an encode and decode round trip unchanged.

use libfuzzer_sys::fuzz_target;
use prost::Message;
use sandbox_proto::framing;
use sandbox_proto::v1::guest::{
    ListDirRequest, ListDirResponse, PingResponse, ReadFileRequest, ReadFileResponse, SyncRequest,
    SyncResponse, WriteFileRequest,
};
use sandbox_proto::v1::types::{ExecRequest, ExecResult};

fn round_trip<T: Message + Default + PartialEq>(bytes: &[u8]) {
    let Ok(message) = T::decode(bytes) else {
        return;
    };
    let encoded = message.encode_to_vec();
    let again = T::decode(encoded.as_slice()).expect("a re-encoded message must decode");
    assert!(message == again, "round trip changed the message");
}

/// The first byte of a payload picks the message type; the rest is its body.
fn dispatch(payload: &[u8]) {
    let Some((&kind, body)) = payload.split_first() else {
        return;
    };
    match kind % 10 {
        0 => round_trip::<PingResponse>(body),
        1 => round_trip::<ReadFileRequest>(body),
        2 => round_trip::<ReadFileResponse>(body),
        3 => round_trip::<WriteFileRequest>(body),
        4 => round_trip::<ListDirRequest>(body),
        5 => round_trip::<ListDirResponse>(body),
        6 => round_trip::<SyncRequest>(body),
        7 => round_trip::<SyncResponse>(body),
        8 => round_trip::<ExecRequest>(body),
        _ => round_trip::<ExecResult>(body),
    }
}

fuzz_target!(|data: &[u8]| {
    // Walk every frame in the input. A frame the decoder rejects ends the walk.
    let mut rest = data;
    while let Ok(Some((payload, used))) = framing::decode(rest) {
        let encoded = framing::encode(payload).expect("a decoded payload is within the cap");
        let (again, _) = framing::decode(&encoded)
            .expect("a re-encoded frame must decode")
            .expect("a re-encoded frame is complete");
        assert_eq!(again, payload, "frame round trip changed the payload");
        dispatch(payload);
        rest = &rest[used..];
    }
    // The whole input is also one message, without a frame header.
    dispatch(data);
});
