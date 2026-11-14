//! Word expansion: variables, positional parameters, field splitting, and glob
//! matching.
//!
//! Output of an unquoted expansion is split into fields on `IFS`, as POSIX sh and
//! bash do. It is not globbed, and it is not expanded again. That is stricter
//! than bash, and it avoids surprises from values that contain `*`. Command
//! substitutions are expanded by the interpreter, which can run them. The
//! interpreter also expands `${...}` and `$((...))`, because those need the
//! shell's state and, for defaults, its substitutions.

use crate::ast::{Ch, Mode, Piece, Word};

/// The field separators of the default `IFS`.
pub const IFS: [char; 3] = [' ', '\t', '\n'];

/// Appends `text` with the given mode.
pub fn push_text(out: &mut Vec<Ch>, text: &str, mode: Mode) {
    out.extend(text.chars().map(|c| Ch { c, mode }));
}

/// Appends each value with a `Break` between them.
pub fn push_values(out: &mut Vec<Ch>, values: &[String], mode: Mode) {
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(Ch {
                c: ' ',
                mode: Mode::Break,
            });
        }
        push_text(out, value, mode);
    }
}

/// Splits expanded characters into fields. A `Break` always ends a field, even an
/// empty one. Whitespace in `Expanded` text ends a field, and runs of it
/// produce no empty fields. Every other character belongs to the field it is in.
/// If the word is quoted and produced no field at all, it gives one empty field,
/// as `''` does.
pub fn split_fields(chars: &[Ch], word_quoted: bool) -> Vec<Vec<Ch>> {
    let mut fields = Vec::new();
    let mut current: Vec<Ch> = Vec::new();
    for &ch in chars {
        match ch.mode {
            Mode::Break => fields.push(std::mem::take(&mut current)),
            Mode::Expanded if IFS.contains(&ch.c) => {
                if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        fields.push(current);
    }
    if fields.is_empty() && word_quoted {
        fields.push(Vec::new());
    }
    fields
}

/// Removes the shortest or longest prefix (`prefix`) or suffix of `value` that
/// matches `pattern`. Returns the remaining text. If nothing matches, the value
/// is returned unchanged.
pub fn trim(value: &[char], pattern: &[Ch], prefix: bool, longest: bool) -> String {
    let n = value.len();
    let matches = |len: usize| {
        if prefix {
            glob_match(pattern, &value[..len])
        } else {
            glob_match(pattern, &value[n - len..])
        }
    };
    // Lengths to try, shortest first, or longest first.
    let lengths: Vec<usize> = if longest {
        (0..=n).rev().collect()
    } else {
        (0..=n).collect()
    };
    for len in lengths {
        if matches(len) {
            let kept = if prefix {
                &value[len..]
            } else {
                &value[..n - len]
            };
            return kept.iter().collect();
        }
    }
    value.iter().collect()
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
            _ => None,
        })
        .collect();
    if !is_name(&name) {
        return None;
    }
    Some((name, Word(word.0[eq + 1..].to_vec())))
}

/// Matches one path segment against a glob pattern. In the pattern, `*`,
/// `?`, and `[...]` are active only when unquoted. Everything else is literal.
///
/// Each (pattern position, name position) pair is solved once. A pattern with
/// many `*` therefore costs O(pattern x name), not exponential in the number of
/// `*`. The naive recursion was exponential, and a 155-byte script ran for over a
/// minute under `fuzz/shell_run`, where the step budget does not count it.
pub fn glob_match(pattern: &[Ch], name: &[char]) -> bool {
    let mut memo = vec![None; (pattern.len() + 1) * (name.len() + 1)];
    glob_match_from(pattern, 0, name, 0, &mut memo)
}

/// Matches `pattern[pi..]` against `name[ni..]`, with `memo` holding every answer
/// solved so far, indexed by `pi * (name.len() + 1) + ni`.
fn glob_match_from(
    pattern: &[Ch],
    pi: usize,
    name: &[char],
    ni: usize,
    memo: &mut [Option<bool>],
) -> bool {
    let key = pi * (name.len() + 1) + ni;
    if let Some(known) = memo[key] {
        return known;
    }
    let result = glob_step(pattern, pi, name, ni, memo);
    memo[key] = Some(result);
    result
}

fn glob_step(
    pattern: &[Ch],
    pi: usize,
    name: &[char],
    ni: usize,
    memo: &mut [Option<bool>],
) -> bool {
    let Some(p) = pattern.get(pi) else {
        return ni == name.len();
    };
    let active = p.mode == Mode::Bare;
    match (active, p.c) {
        // `*` matches nothing, or one more character of the name and then `*` again.
        (true, '*') => {
            glob_match_from(pattern, pi + 1, name, ni, memo)
                || (ni < name.len() && glob_match_from(pattern, pi, name, ni + 1, memo))
        }
        (true, '?') => ni < name.len() && glob_match_from(pattern, pi + 1, name, ni + 1, memo),
        (true, '[') => match pattern[pi + 1..]
            .iter()
            .position(|c| c.mode == Mode::Bare && c.c == ']')
        {
            Some(offset) => {
                let close = pi + 1 + offset;
                let class = &pattern[pi + 1..close];
                let (negate, items) = match class.first() {
                    Some(first) if first.mode == Mode::Bare && matches!(first.c, '!' | '^') => {
                        (true, &class[1..])
                    }
                    _ => (false, class),
                };
                match name.get(ni) {
                    Some(&c) => {
                        class_contains(items, c) != negate
                            && glob_match_from(pattern, close + 1, name, ni + 1, memo)
                    }
                    None => false,
                }
            }
            // An unclosed `[` is a literal bracket.
            None => {
                name.get(ni) == Some(&'[') && glob_match_from(pattern, pi + 1, name, ni + 1, memo)
            }
        },
        _ => name.get(ni) == Some(&p.c) && glob_match_from(pattern, pi + 1, name, ni + 1, memo),
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
