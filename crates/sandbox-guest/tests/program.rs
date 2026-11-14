use sandbox_guest::program::{in_system_dir, quote};

#[test]
fn quoting_survives_quotes_and_spaces() {
    assert_eq!(quote("/a b"), "'/a b'");
    assert_eq!(quote("it's"), "'it'\\''s'");
}

#[test]
fn the_system_directory_is_recognised() {
    assert!(in_system_dir("/bin"));
    assert!(in_system_dir("/bin/busybox"));
    assert!(!in_system_dir("/binary/x"));
    assert!(!in_system_dir("/work"));
}
