//! Word expansion: variables, positional parameters, assignments, and glob matching.
//!
//! Expanded values are literal. They are not globbed, and they are not
//! expanded again. That is stricter than POSIX sh and avoids surprises from
//! values that contain `*`. Command substitutions are expanded by the
//! interpreter, which can run them. This module handles everything else.

use crate::ast::{Ch, Mode, Piece, Word};
use std::collections::HashMap;

/// What variable expansion can see.
pub struct Scope<'a> {
    pub env: &'a HashMap<String, String>,
    /// `$1`, `$2`, ... as set for the current function call.
    pub positional: &'a [String],
    /// `$?`
    pub status: i32,
}

/// The characters of a word's `Ch` pieces, in order. Substitutions are skipped.
pub fn plain_chars(word: &Word) -> Vec<Ch> {
    word.0
        .iter()
        .filter_map(|piece| match piece {
            Piece::Ch(ch) => Some(*ch),
            Piece::Subst(_) => None,
        })
        .collect()
}

/// Replaces `$NAME`, `${NAME}`, `$?`, `$#`, `$@`, `$*`, and `$N` in unquoted
/// and double-quoted text. Single-quoted text is left alone. An unset
/// variable expands to nothing.
pub fn expand_vars(chars: &[Ch], scope: &Scope<'_>) -> Vec<Ch> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.c != '$' || ch.mode == Mode::Literal {
            out.push(ch);
            i += 1;
            continue;
        }
        let expandable = |c: &Ch| c.mode != Mode::Literal;
        match chars.get(i + 1) {
            Some(next) if expandable(next) && matches!(next.c, '?' | '#' | '@' | '*') => {
                let value = special(scope, &next.c.to_string()).unwrap_or_default();
                push_literal(&mut out, &value);
                i += 2;
            }
            Some(next) if expandable(next) && next.c.is_ascii_digit() => {
                let index = next.c.to_digit(10).unwrap_or(0) as usize;
                push_literal(&mut out, &positional(scope, index));
                i += 2;
            }
            Some(next) if next.c == '{' && expandable(next) => {
                let close = chars[i + 2..]
                    .iter()
                    .position(|c| c.c == '}' && expandable(c))
                    .map(|p| p + i + 2);
                match close {
                    Some(end) => {
                        let name: String = chars[i + 2..end].iter().map(|c| c.c).collect();
                        if is_name(&name) {
                            push_literal(&mut out, &lookup(scope, &name));
                        } else if let Some(value) = special(scope, &name) {
                            push_literal(&mut out, &value);
                        } else if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
                            let index: usize = name.parse().unwrap_or(usize::MAX);
                            push_literal(&mut out, &positional(scope, index));
                        } else {
                            out.push(ch);
                            i += 1;
                            continue;
                        }
                        i = end + 1;
                    }
                    None => {
                        out.push(ch);
                        i += 1;
                    }
                }
            }
            Some(next) if expandable(next) && is_name_start(next.c) => {
                let mut end = i + 1;
                while end < chars.len() && expandable(&chars[end]) && is_name_char(chars[end].c) {
                    end += 1;
                }
                let name: String = chars[i + 1..end].iter().map(|c| c.c).collect();
                push_literal(&mut out, &lookup(scope, &name));
                i = end;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// Values of the special parameters `?`, `#`, `@`, and `*`.
fn special(scope: &Scope<'_>, name: &str) -> Option<String> {
    match name {
        "?" => Some(scope.status.to_string()),
        "#" => Some(scope.positional.len().to_string()),
        "@" | "*" => Some(scope.positional.join(" ")),
        _ => None,
    }
}

/// `$0` is the shell name. `$1`..`$n` are the function's arguments.
fn positional(scope: &Scope<'_>, index: usize) -> String {
    match index {
        0 => "sandbox".to_string(),
        n => scope.positional.get(n - 1).cloned().unwrap_or_default(),
    }
}

fn lookup(scope: &Scope<'_>, name: &str) -> String {
    scope.env.get(name).cloned().unwrap_or_default()
}

fn push_literal(out: &mut Vec<Ch>, text: &str) {
    out.extend(text.chars().map(|c| Ch {
        c,
        mode: Mode::Literal,
    }));
}

fn is_name_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_name_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Whether `text` is a valid variable name.
pub fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if is_name_start(c)) && chars.all(is_name_char)
}

/// Whether the characters contain an unquoted glob character.
pub fn has_glob(chars: &[Ch]) -> bool {
    chars
        .iter()
        .any(|ch| ch.mode == Mode::Bare && matches!(ch.c, '*' | '?' | '['))
}

/// The text of expanded characters (quotes already removed).
pub fn text(chars: &[Ch]) -> String {
    chars.iter().map(|ch| ch.c).collect()
}

/// If `word` has the form `NAME=value` with an unquoted name, returns the
/// name and the value as an unexpanded word.
pub fn split_assignment(word: &Word) -> Option<(String, Word)> {
    let eq = word
        .0
        .iter()
        .position(|piece| matches!(piece, Piece::Ch(ch) if ch.mode == Mode::Bare && ch.c == '='))?;
    let name_pieces = &word.0[..eq];
    if name_pieces
        .iter()
        .any(|p| !matches!(p, Piece::Ch(ch) if ch.mode == Mode::Bare))
    {
        return None;
    }
    let name: String = name_pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Ch(ch) => Some(ch.c),
            Piece::Subst(_) => None,
        })
        .collect();
    if !is_name(&name) {
        return None;
    }
    Some((name, Word(word.0[eq + 1..].to_vec())))
}

/// Matches one path segment against a glob pattern. In the pattern, `*`,
/// `?`, and `[...]` are active only when unquoted. Everything else is literal.
pub fn glob_match(pattern: &[Ch], name: &[char]) -> bool {
    let Some((p, rest)) = pattern.split_first() else {
        return name.is_empty();
    };
    let active = p.mode == Mode::Bare;
    match (active, p.c) {
        (true, '*') => (0..=name.len()).any(|i| glob_match(rest, &name[i..])),
        (true, '?') => !name.is_empty() && glob_match(rest, &name[1..]),
        (true, '[') => match rest.iter().position(|c| c.mode == Mode::Bare && c.c == ']') {
            Some(end) => {
                let class = &rest[..end];
                let (negate, items) = match class.first() {
                    Some(first) if first.mode == Mode::Bare && matches!(first.c, '!' | '^') => {
                        (true, &class[1..])
                    }
                    _ => (false, class),
                };
                match name.first() {
                    Some(&c) => {
                        class_contains(items, c) != negate
                            && glob_match(&rest[end + 1..], &name[1..])
                    }
                    None => false,
                }
            }
            // An unclosed `[` is a literal bracket.
            None => name.first() == Some(&'[') && glob_match(rest, &name[1..]),
        },
        _ => name.first() == Some(&p.c) && glob_match(rest, &name[1..]),
    }
}

fn class_contains(items: &[Ch], c: char) -> bool {
    let mut i = 0;
    while i < items.len() {
        if i + 2 < items.len() && items[i + 1].mode == Mode::Bare && items[i + 1].c == '-' {
            if items[i].c <= c && c <= items[i + 2].c {
                return true;
            }
            i += 3;
        } else {
            if items[i].c == c {
                return true;
            }
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn expand(
        input: &[Ch],
        env: &HashMap<String, String>,
        status: i32,
        positional: &[String],
    ) -> String {
        text(&expand_vars(
            input,
            &Scope {
                env,
                positional,
                status,
            },
        ))
    }

    #[test]
    fn expands_plain_and_braced_variables() {
        let e = env(&[("X", "1"), ("NAME", "bob")]);
        assert_eq!(
            expand(&chars(&[("a$X-${NAME}", Mode::Bare)]), &e, 0, &[]),
            "a1-bob"
        );
        assert_eq!(expand(&chars(&[("$MISSING", Mode::Bare)]), &e, 0, &[]), "");
    }

    #[test]
    fn single_quotes_block_expansion_and_double_quotes_allow_it() {
        let e = env(&[("X", "v")]);
        assert_eq!(expand(&chars(&[("$X", Mode::Literal)]), &e, 0, &[]), "$X");
        assert_eq!(expand(&chars(&[("$X", Mode::Double)]), &e, 0, &[]), "v");
    }

    #[test]
    fn special_parameters_expand() {
        let e = env(&[]);
        let args = vec!["one".to_string(), "two".to_string()];
        assert_eq!(expand(&chars(&[("$?", Mode::Bare)]), &e, 7, &[]), "7");
        assert_eq!(expand(&chars(&[("$#", Mode::Bare)]), &e, 0, &args), "2");
        assert_eq!(
            expand(&chars(&[("$1-$2", Mode::Bare)]), &e, 0, &args),
            "one-two"
        );
        assert_eq!(
            expand(&chars(&[("$@", Mode::Bare)]), &e, 0, &args),
            "one two"
        );
        assert_eq!(expand(&chars(&[("$3", Mode::Bare)]), &e, 0, &args), "");
        assert_eq!(expand(&chars(&[("$0", Mode::Bare)]), &e, 0, &[]), "sandbox");
    }

    #[test]
    fn bad_names_are_literal() {
        let e = env(&[]);
        // `$1x` is `$1` followed by a literal `x`, as in POSIX sh.
        assert_eq!(
            expand(&chars(&[("$1x", Mode::Bare)]), &e, 0, &["a".to_string()]),
            "ax"
        );
        assert_eq!(expand(&chars(&[("$1x", Mode::Bare)]), &e, 0, &[]), "x");
        assert_eq!(
            expand(&chars(&[("${bad-name}", Mode::Bare)]), &e, 0, &[]),
            "${bad-name}"
        );
    }

    #[test]
    fn assignments_need_an_unquoted_name() {
        let (name, value) = split_assignment(&word(&[("PATH=/bin", Mode::Bare)])).unwrap();
        assert_eq!(
            (name.as_str(), text(&plain_chars(&value)).as_str()),
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
}
