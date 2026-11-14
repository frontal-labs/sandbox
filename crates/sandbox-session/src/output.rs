//! Output capping for buffered command results (IMAGE.md §4.5).
//!
//! A command can print far more than an agent can read. The result keeps the start
//! and the end of each stream and replaces the middle with one marker line. The
//! complete stream stays in the session's trace, and the marker names where to
//! read it.

/// The default `output_limit`: 8 KiB kept from the start and 8 KiB from the end of
/// each stream.
pub const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;

/// Keeps the first and last `limit / 2` bytes of `data` and replaces the bytes in
/// between with one marker line. Output within `limit` is returned unchanged.
///
/// `full` is where the complete stream can be read. `None` means the stream was not
/// recorded, so the marker says so instead of naming a path that does not exist.
/// The line count covers the omitted bytes. A partial last line counts as one line.
pub fn cap_output(data: &[u8], limit: usize, full: Option<&str>) -> Vec<u8> {
    if data.len() <= limit {
        return data.to_vec();
    }
    let head_len = limit / 2;
    let tail_len = limit - head_len;
    let middle = &data[head_len..data.len() - tail_len];
    let mut lines = middle.iter().filter(|&&byte| byte == b'\n').count();
    if !middle.ends_with(b"\n") {
        lines += 1;
    }
    let location = match full {
        Some(path) => format!(" · full: {path}"),
        None => " · full output not recorded".to_string(),
    };
    let marker = format!("…[{lines} lines omitted{location}]\n");

    let mut out = Vec::with_capacity(limit + marker.len() + 1);
    out.extend_from_slice(&data[..head_len]);
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(marker.as_bytes());
    out.extend_from_slice(&data[data.len() - tail_len..]);
    out
}
