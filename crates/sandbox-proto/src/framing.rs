//! Length-prefixed frames for byte streams (the guest protocol, project specification section 4.10).
//!
//! A frame is a 4-byte big-endian length followed by that many payload bytes.
//! The length is checked against [`MAX_FRAME_BYTES`] before any allocation, so a
//! hostile peer cannot make the reader allocate more than the cap.

use sandbox_types::{ErrorCode, SandboxError};

/// Largest payload accepted in one frame (16 MiB).
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

const HEADER_BYTES: usize = 4;

/// Encodes one payload as a frame.
pub fn encode(payload: &[u8]) -> Result<Vec<u8>, SandboxError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(too_large(payload.len()));
    }
    let len = u32::try_from(payload.len()).map_err(|_| too_large(payload.len()))?;
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Decodes the first frame in `buf`. Returns the payload and the bytes consumed.
/// Returns `Ok(None)` if `buf` does not yet hold a complete frame.
pub fn decode(buf: &[u8]) -> Result<Option<(&[u8], usize)>, SandboxError> {
    if buf.len() < HEADER_BYTES {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(too_large(len));
    }
    let end = HEADER_BYTES + len;
    if buf.len() < end {
        return Ok(None);
    }
    Ok(Some((&buf[HEADER_BYTES..end], end)))
}

pub(crate) fn too_large(len: usize) -> SandboxError {
    SandboxError::new(
        ErrorCode::ResourceExhausted,
        format!("frame of {len} bytes exceeds the {MAX_FRAME_BYTES} byte limit"),
    )
}
