use sandbox_shell::printf::format;

fn fmt(format_text: &str, args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let formatted = format(format_text, &args).unwrap();
    assert!(formatted.error.is_none(), "{:?}", formatted.error);
    String::from_utf8(formatted.output).unwrap()
}

#[test]
fn strings_and_escapes() {
    assert_eq!(fmt("%s\\n", &["word"]), "word\n");
    assert_eq!(fmt("%s-%s\\n", &["a", "b", "c", "d"]), "a-b\nc-d\n");
    assert_eq!(fmt("x", &[]), "x");
    assert_eq!(fmt("%%\\n", &[]), "%\n");
}

#[test]
fn widths_and_alignment() {
    assert_eq!(fmt("%5s|", &["ab"]), "   ab|");
    assert_eq!(fmt("%-5s|", &["ab"]), "ab   |");
}

#[test]
fn numbers_and_bases() {
    assert_eq!(fmt("%d", &["42"]), "42");
    assert_eq!(fmt("%x", &["255"]), "ff");
    assert_eq!(fmt("%X", &["255"]), "FF");
    assert_eq!(fmt("%o", &["8"]), "10");
    assert_eq!(fmt("%05d", &["-42"]), "-0042");
    assert_eq!(fmt("%+d", &["7"]), "+7");
}

#[test]
fn characters_and_missing_arguments() {
    assert_eq!(fmt("%c", &["Z"]), "Z");
    assert_eq!(fmt("[%s]", &[]), "[]");
    assert_eq!(fmt("[%d]", &[]), "[0]");
}

#[test]
fn the_format_is_reused_for_remaining_arguments() {
    assert_eq!(fmt("%s\\n", &["one", "two"]), "one\ntwo\n");
}

#[test]
fn a_bad_number_formats_as_zero_and_is_reported() {
    let args = vec!["abc".to_string()];
    let formatted = format("[%d]", &args).unwrap();
    assert_eq!(formatted.output, b"[0]");
    assert!(formatted.error.unwrap().contains("invalid number"));
}

#[test]
fn an_unsupported_directive_is_an_error() {
    assert!(format("%f", &[]).unwrap_err().contains("unsupported"));
}

#[test]
fn percent_b_expands_escapes_in_the_argument() {
    assert_eq!(fmt("%b", &["a\\tb"]), "a\tb");
}
