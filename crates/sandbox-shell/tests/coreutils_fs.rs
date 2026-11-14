use sandbox_shell::coreutils_fs::{glob_match, relative_path, xargs_words};

#[test]
fn glob_matches_stars_and_question_marks() {
    assert!(glob_match(b"*.txt", b"notes.txt"));
    assert!(glob_match(b"a?c", b"abc"));
    assert!(!glob_match(b"*.txt", b"notes.md"));
    assert!(glob_match(b"*", b""));
}

#[test]
fn xargs_splits_on_blanks_and_honours_quotes() {
    assert_eq!(xargs_words(b"a  b\nc").unwrap(), vec!["a", "b", "c"]);
    assert_eq!(xargs_words(b"'x y' z").unwrap(), vec!["x y", "z"]);
    assert!(xargs_words(b"'open").is_err());
}

#[test]
fn relative_path_climbs_to_the_common_ancestor() {
    assert_eq!(relative_path("/a/b", "/a/c/d"), "../c/d");
    assert_eq!(relative_path("/a", "/a"), ".");
}
