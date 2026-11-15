use sandbox_shell::ast::{Ch, Mode, Piece, Word};
use sandbox_shell::expand::{glob_match, has_glob, split_assignment, text};

fn bare(s: &str) -> Vec<Ch> {
    s.chars()
        .map(|c| Ch {
            c,
            mode: Mode::Bare,
        })
        .collect()
}

fn chars(parts: &[(&str, Mode)]) -> Vec<Ch> {
    parts
        .iter()
        .flat_map(|(s, m)| s.chars().map(move |c| Ch { c, mode: *m }))
        .collect()
}

fn word(parts: &[(&str, Mode)]) -> Word {
    Word(chars(parts).into_iter().map(Piece::Ch).collect())
}

/// The characters of a word's `Ch` pieces, for checking a split value.
fn value_chars(word: &Word) -> Vec<Ch> {
    word.0
        .iter()
        .filter_map(|piece| match piece {
            Piece::Ch(ch) => Some(*ch),
            _ => None,
        })
        .collect()
}

#[test]
fn assignments_need_an_unquoted_name() {
    let (name, value) = split_assignment(&word(&[("PATH=/bin", Mode::Bare)])).unwrap();
    assert_eq!(
        (name.as_str(), text(&value_chars(&value)).as_str()),
        ("PATH", "/bin")
    );
    assert!(split_assignment(&word(&[("'A'=1", Mode::Literal)])).is_none());
    assert!(split_assignment(&word(&[("1A=1", Mode::Bare)])).is_none());
    assert!(split_assignment(&word(&[("echo", Mode::Bare)])).is_none());
}

#[test]
fn glob_patterns_match_names() {
    let m = |pat: &str, name: &str| {
        let pattern = bare(pat);
        let name: Vec<char> = name.chars().collect();
        glob_match(&pattern, &name)
    };
    assert!(m("*.txt", "notes.txt"));
    assert!(!m("*.txt", "notes.md"));
    assert!(m("f?o", "foo"));
    assert!(m("[a-c]x", "bx"));
    assert!(!m("[!a-c]x", "bx"));
    assert!(m("[!a-c]x", "dx"));
    assert!(m("a[", "a["));
}

#[test]
fn quoted_glob_characters_are_literal() {
    let pattern = chars(&[("*", Mode::Literal)]);
    assert!(glob_match(&pattern, &['*']));
    assert!(!glob_match(&pattern, &['a']));
    assert!(!has_glob(&pattern));
}
