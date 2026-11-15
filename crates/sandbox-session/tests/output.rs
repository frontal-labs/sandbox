//! Integration tests for output capping, moved out of src/output.rs.

use sandbox_session::cap_output;

#[test]
fn output_within_the_limit_is_returned_unchanged() {
    let data = b"hello\nworld\n";
    assert_eq!(cap_output(data, data.len(), Some("/x")), data.to_vec());
    assert_eq!(cap_output(data, 1024, None), data.to_vec());
}

#[test]
fn a_long_stream_keeps_its_start_and_end_and_one_marker() {
    let lines: String = (0..100_000).map(|i| format!("line {i}\n")).collect();
    let data = lines.as_bytes();
    let limit = 4096;
    let capped = cap_output(data, limit, Some("/trace/exec/7.out"));
    let text = String::from_utf8(capped.clone()).unwrap();

    assert!(
        capped.len() < limit + 200,
        "capped to {} bytes",
        capped.len()
    );
    assert!(text.starts_with("line 0\n"));
    assert!(text.ends_with("line 99999\n"));
    assert_eq!(text.matches("lines omitted").count(), 1);
    assert!(text.contains("· full: /trace/exec/7.out]"));
}

#[test]
fn the_marker_counts_the_omitted_lines() {
    // 10 lines of 10 bytes each. A limit of 40 keeps 20 bytes from each end,
    // which is two lines. Six lines are omitted.
    let data: Vec<u8> = (0..10).flat_map(|_| b"123456789\n".to_vec()).collect();
    let capped = cap_output(&data, 40, Some("/trace/exec/1.out"));
    let text = String::from_utf8(capped).unwrap();
    assert!(
        text.contains("…[6 lines omitted · full: /trace/exec/1.out]"),
        "{text}"
    );
}

#[test]
fn a_stream_without_newlines_still_reports_a_line_omitted() {
    let data = vec![b'x'; 100];
    let capped = cap_output(&data, 20, Some("/trace/exec/2.err"));
    let text = String::from_utf8(capped).unwrap();
    assert!(
        text.contains("…[1 lines omitted · full: /trace/exec/2.err]"),
        "{text}"
    );
}

#[test]
fn an_unrecorded_stream_says_so_and_names_no_path() {
    let data = vec![b'a'; 64];
    let capped = cap_output(&data, 16, None);
    let text = String::from_utf8(capped).unwrap();
    assert!(text.contains("full output not recorded"), "{text}");
    assert!(!text.contains("/trace"), "{text}");
}

#[test]
fn a_zero_limit_keeps_only_the_marker() {
    let capped = cap_output(b"abc\n", 0, Some("/trace/exec/3.out"));
    let text = String::from_utf8(capped).unwrap();
    assert!(text.starts_with("…[1 lines omitted"), "{text}");
    assert!(text.ends_with("]\n"), "{text}");
}
