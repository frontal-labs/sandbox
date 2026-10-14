//! Substitution nesting is capped. Each level recurses into the parser, and an
//! unbounded depth overflows the stack, which aborts the process.

use sandbox_types::ErrorCode;

fn nested(depth: usize) -> String {
    let mut s = String::from("echo ");
    for _ in 0..depth {
        s.push_str("$(");
    }
    s.push('x');
    for _ in 0..depth {
        s.push(')');
    }
    s
}

#[test]
fn ordinary_nesting_still_parses() {
    assert!(sandbox_shell::parse(&nested(4)).is_ok());
}

#[test]
fn deep_nesting_is_refused_without_overflowing() {
    for depth in [33usize, 5_000, 50_000] {
        let err = sandbox_shell::parse(&nested(depth)).unwrap_err();
        assert_eq!(err.code, ErrorCode::ResourceExhausted, "depth {depth}");
    }
}

#[test]
fn depth_counter_recovers_after_an_error() {
    assert!(sandbox_shell::parse(&nested(50_000)).is_err());
    // The counter must unwind, or every later parse would fail.
    assert!(sandbox_shell::parse(&nested(4)).is_ok());
}
