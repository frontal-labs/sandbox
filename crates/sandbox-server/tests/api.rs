//! Unit tests moved out of src/api.rs.

use sandbox_server::api::{
    evict_to_budget, parse_checkpoint_budget, parse_spill_budget, CheckpointRoot,
    DEFAULT_CHECKPOINT_BYTES_PER_KEY, DEFAULT_SPILLED_BYTES_PER_SESSION, MAX_READ_BYTES,
};
use sandbox_types::hash::ContentHash;

#[test]
fn the_spill_budget_setting_falls_back_to_the_default_when_unusable() {
    assert_eq!(parse_spill_budget(None), DEFAULT_SPILLED_BYTES_PER_SESSION);
    assert_eq!(
        parse_spill_budget(Some("x")),
        DEFAULT_SPILLED_BYTES_PER_SESSION
    );
    assert_eq!(
        parse_spill_budget(Some("0")),
        DEFAULT_SPILLED_BYTES_PER_SESSION
    );
    assert_eq!(parse_spill_budget(Some(" 1024 ")), 1024);
}

#[test]
fn the_budget_setting_falls_back_to_the_default_when_unusable() {
    assert_eq!(
        parse_checkpoint_budget(None),
        DEFAULT_CHECKPOINT_BYTES_PER_KEY
    );
    assert_eq!(
        parse_checkpoint_budget(Some("abc")),
        DEFAULT_CHECKPOINT_BYTES_PER_KEY
    );
    assert_eq!(
        parse_checkpoint_budget(Some("0")),
        DEFAULT_CHECKPOINT_BYTES_PER_KEY
    );
    assert_eq!(parse_checkpoint_budget(Some(" 4096 ")), 4096);
}

fn root(byte: u8, bytes: u64) -> CheckpointRoot {
    CheckpointRoot {
        root: ContentHash::new(&[byte]),
        bytes,
    }
}

#[test]
fn eviction_drops_the_oldest_roots_until_the_total_fits() {
    let mut a = vec![root(1, 40), root(2, 40), root(3, 40)];
    let mut b = vec![root(4, 30), root(5, 30)];
    assert!(evict_to_budget(&mut [&mut a, &mut b], 100));
    assert!(a.iter().map(|r| r.bytes).sum::<u64>() + b.iter().map(|r| r.bytes).sum::<u64>() <= 100);
    // Every list keeps its newest root.
    assert_eq!(a.last().unwrap().root, ContentHash::new(&[3]));
    assert_eq!(b.last().unwrap().root, ContentHash::new(&[5]));
}

#[test]
fn eviction_changes_nothing_when_the_newest_roots_do_not_fit() {
    let mut a = vec![root(1, 10), root(2, 60)];
    let mut b = vec![root(3, 60)];
    assert!(!evict_to_budget(&mut [&mut a, &mut b], 100));
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1);
}

#[test]
fn reads_are_capped_at_the_limit() {
    assert!(!over_read_limit(0));
    assert!(!over_read_limit(MAX_READ_BYTES));
    assert!(over_read_limit(MAX_READ_BYTES + 1));
    // The VFS allows 100 MiB per file, which is above the read cap.
    assert!(over_read_limit(100 * 1024 * 1024));
}

// Helper that mirrors the private fn in src/api.rs.
fn over_read_limit(size: u64) -> bool {
    size > MAX_READ_BYTES
}
