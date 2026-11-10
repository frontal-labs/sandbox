//! Coreutils for text: `sed` (the substitute command), `diff` (normal format), `nl`,
//! `base64`, and `date`. Each one matches GNU for the options it accepts. Anything else
//! is refused, so a script never gets quietly different output.

use crate::builtins::ok;
use crate::interpreter::{failure, ExecutionResult, Shell};
use chrono::{DateTime, Utc};

/// Splits data into lines without their newlines. Same rule as `coreutils::split_lines`.
fn lines_of(data: &[u8]) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    let body = data.strip_suffix(b"\n").unwrap_or(data);
    body.split(|&b| b == b'\n').collect()
}

/// Translates a POSIX basic regular expression to the syntax the `regex` crate reads.
/// In a BRE, `\( \) \{ \} \| \+ \?` are the operators and the bare characters are
/// literal, which is the reverse of ERE. ERE patterns pass through unchanged.
fn bre_to_regex(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(op @ ('(' | ')' | '{' | '}' | '|' | '+' | '?')) => out.push(op),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push_str("\\\\"),
            }
        } else if matches!(c, '(' | ')' | '{' | '}' | '|' | '+' | '?') {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// One `s/REGEX/REPLACEMENT/FLAGS` command.
struct Substitute {
    regex: regex::Regex,
    replacement: String,
    global: bool,
    print: bool,
}

/// Parses `s<d>RE<d>REP<d>[gpi]` where `<d>` is any delimiter character. A backslash
/// before the delimiter makes it literal.
fn parse_substitute(script: &str, extended: bool) -> Result<Substitute, String> {
    let mut chars = script.chars();
    if chars.next() != Some('s') {
        return Err("sed: only the s command is supported".to_string());
    }
    let delim = chars
        .next()
        .ok_or_else(|| "sed: unterminated s command".to_string())?;
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    while parts.len() < 2 {
        match chars.next() {
            None => return Err("sed: unterminated s command".to_string()),
            Some('\\') => match chars.next() {
                Some(d) if d == delim => current.push(d),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => return Err("sed: unterminated s command".to_string()),
            },
            Some(c) if c == delim => parts.push(std::mem::take(&mut current)),
            Some(c) => current.push(c),
        }
    }
    let (pattern, replacement) = (parts.remove(0), parts.remove(0));
    let mut global = false;
    let mut print = false;
    let mut case_insensitive = false;
    for f in chars {
        match f {
            'g' => global = true,
            'p' => print = true,
            'i' | 'I' => case_insensitive = true,
            other => return Err(format!("sed: unsupported s flag '{other}'")),
        }
    }
    let source = if extended {
        pattern
    } else {
        bre_to_regex(&pattern)
    };
    let regex = regex::RegexBuilder::new(&source)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("sed: invalid regular expression: {e}"))?;
    Ok(Substitute {
        regex,
        replacement,
        global,
        print,
    })
}

/// Applies a replacement template: `&` is the match, `\1`..`\9` are groups, `\n` is a
/// newline, and any other escaped character stands for itself.
fn expand_replacement(template: &str, caps: &regex::Captures) -> String {
    let mut out = String::new();
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str(caps.get(0).map_or("", |m| m.as_str())),
            '\\' => match chars.next() {
                Some(d @ '1'..='9') => {
                    let index = d.to_digit(10).unwrap_or(0) as usize;
                    out.push_str(caps.get(index).map_or("", |m| m.as_str()));
                }
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

/// One line-level edit of an LCS diff.
#[derive(Clone, Copy, PartialEq)]
enum Edit {
    Same,
    Delete(usize),
    Insert(usize),
}

/// Edits turning `a` into `b`, from a longest common subsequence.
fn diff_edits(a: &[&[u8]], b: &[&[u8]]) -> Vec<Edit> {
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut edits = Vec::new();
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            edits.push(Edit::Same);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            edits.push(Edit::Insert(j));
            j += 1;
        } else {
            edits.push(Edit::Delete(i));
            i += 1;
        }
    }
    edits
}

/// `a`, `b`, or `c` hunk header with its GNU range text, e.g. `2`, `2,3`.
fn range_text(first: usize, count: usize) -> String {
    if count == 1 {
        format!("{first}")
    } else {
        format!("{first},{}", first + count - 1)
    }
}

/// Renders the GNU normal-format diff of two line lists.
fn normal_diff(a: &[&[u8]], b: &[&[u8]]) -> String {
    let edits = diff_edits(a, b);
    let mut out = String::new();
    let mut k = 0;
    while k < edits.len() {
        if edits[k] == Edit::Same {
            k += 1;
            continue;
        }
        let start = k;
        while k < edits.len() && edits[k] != Edit::Same {
            k += 1;
        }
        let deleted: Vec<usize> = edits[start..k]
            .iter()
            .filter_map(|e| match e {
                Edit::Delete(i) => Some(*i),
                _ => None,
            })
            .collect();
        let inserted: Vec<usize> = edits[start..k]
            .iter()
            .filter_map(|e| match e {
                Edit::Insert(j) => Some(*j),
                _ => None,
            })
            .collect();
        // The old and new line numbers just before this hunk, 1-based.
        let old_before = edits[..start]
            .iter()
            .filter(|e| !matches!(e, Edit::Insert(_)))
            .count();
        let new_before = edits[..start]
            .iter()
            .filter(|e| !matches!(e, Edit::Delete(_)))
            .count();
        match (deleted.is_empty(), inserted.is_empty()) {
            (false, true) => {
                out.push_str(&format!(
                    "{}d{new_before}\n",
                    range_text(deleted[0] + 1, deleted.len())
                ));
                for i in &deleted {
                    out.push_str(&format!("< {}\n", String::from_utf8_lossy(a[*i])));
                }
            }
            (true, false) => {
                out.push_str(&format!(
                    "{old_before}a{}\n",
                    range_text(inserted[0] + 1, inserted.len())
                ));
                for j in &inserted {
                    out.push_str(&format!("> {}\n", String::from_utf8_lossy(b[*j])));
                }
            }
            _ => {
                out.push_str(&format!(
                    "{}c{}\n",
                    range_text(deleted[0] + 1, deleted.len()),
                    range_text(inserted[0] + 1, inserted.len())
                ));
                for i in &deleted {
                    out.push_str(&format!("< {}\n", String::from_utf8_lossy(a[*i])));
                }
                out.push_str("---\n");
                for j in &inserted {
                    out.push_str(&format!("> {}\n", String::from_utf8_lossy(b[*j])));
                }
            }
        }
    }
    out
}

impl Shell {
    /// `sed [-n] [-E] 's/RE/REP/[gpi]' [FILE...]`. Only the substitute command is
    /// supported; `-n` makes the `p` flag the only way a line is printed.
    pub(crate) async fn sed(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut quiet = false;
        let mut extended = false;
        let mut operands: Vec<String> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-n" => quiet = true,
                "-E" | "-r" => extended = true,
                other if other.starts_with('-') && other.len() > 1 => {
                    return failure(format!("sed: unsupported option '{other}'"))
                }
                _ => operands.push(arg.clone()),
            }
        }
        let Some(script) = operands.first().cloned() else {
            return failure("sed: missing script".to_string());
        };
        let command = match parse_substitute(&script, extended) {
            Ok(c) => c,
            Err(e) => return failure(e),
        };
        let inputs = self.read_inputs("sed", &operands[1..], stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        let mut out = Vec::new();
        for line in lines_of(&data) {
            let text = String::from_utf8_lossy(line).into_owned();
            let mut replaced = false;
            let result = if command.global {
                command
                    .regex
                    .replace_all(&text, |caps: &regex::Captures| {
                        replaced = true;
                        expand_replacement(&command.replacement, caps)
                    })
                    .into_owned()
            } else {
                command
                    .regex
                    .replace(&text, |caps: &regex::Captures| {
                        replaced = true;
                        expand_replacement(&command.replacement, caps)
                    })
                    .into_owned()
            };
            let print_line = if quiet {
                command.print && replaced
            } else {
                true
            };
            if print_line {
                out.extend_from_slice(result.as_bytes());
                out.push(b'\n');
                if command.print && !quiet && replaced {
                    out.extend_from_slice(result.as_bytes());
                    out.push(b'\n');
                }
            }
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `diff FILE1 FILE2`: the GNU normal format. Exit status is 0 when the files are
    /// the same, 1 when they differ, and 2 on error.
    pub(crate) async fn diff(&self, args: &[String]) -> ExecutionResult {
        if args.len() != 2 || args.iter().any(|a| a.starts_with('-')) {
            return failure(
                "diff: only two file operands in normal format are supported".to_string(),
            );
        }
        let a = match self.read_file(&args[0]).await {
            Ok(d) => d,
            Err(e) => {
                return failure(format!(
                    "diff: {}: {}",
                    args[0],
                    crate::interpreter::describe(&e)
                ))
            }
        };
        let b = match self.read_file(&args[1]).await {
            Ok(d) => d,
            Err(e) => {
                return failure(format!(
                    "diff: {}: {}",
                    args[1],
                    crate::interpreter::describe(&e)
                ))
            }
        };
        let (la, lb) = (lines_of(&a), lines_of(&b));
        let text = normal_diff(&la, &lb);
        ExecutionResult {
            exit_code: i32::from(!text.is_empty()),
            stdout: text.into_bytes(),
            stderr: Vec::new(),
        }
    }

    /// `nl [-b a|t]`: numbers lines. The default numbers only non-empty lines, as GNU
    /// does, and `-b a` numbers every line. The number is six wide, then a tab.
    pub(crate) async fn nl(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut all = false;
        let mut operands = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-b" {
                match iter.next().map(|v| v.as_str()) {
                    Some("a") => all = true,
                    Some("t") => all = false,
                    _ => return failure("nl: only -b a and -b t are supported".to_string()),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("nl: unsupported option '{arg}'"));
            } else {
                operands.push(arg.clone());
            }
        }
        let inputs = self.read_inputs("nl", &operands, stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        let mut out = Vec::new();
        let mut number = 0u64;
        for line in lines_of(&data) {
            if all || !line.is_empty() {
                number += 1;
                out.extend_from_slice(format!("{number:>6}\t").as_bytes());
            } else {
                out.extend_from_slice(b"       ");
            }
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `base64 [-d] [-w N] [FILE]`: encodes or decodes standard base64. Encoded output
    /// wraps every `N` characters (76 by default, 0 for no wrapping).
    pub(crate) async fn base64(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        use base64::Engine;
        let mut decode = false;
        let mut wrap = 76usize;
        let mut operands = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-d" || arg == "--decode" {
                decode = true;
            } else if arg == "-w" {
                wrap = match iter.next().and_then(|v| v.parse().ok()) {
                    Some(n) => n,
                    None => return failure("base64: invalid wrap size".to_string()),
                };
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("base64: unsupported option '{arg}'"));
            } else {
                operands.push(arg.clone());
            }
        }
        let inputs = self.read_inputs("base64", &operands, stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        if decode {
            let compact: Vec<u8> = data
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            return match base64::engine::general_purpose::STANDARD.decode(&compact) {
                Ok(bytes) => ok(bytes),
                Err(_) => failure("base64: invalid input".to_string()),
            };
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let mut out = String::new();
        if wrap == 0 {
            // GNU writes no newline after an unwrapped encoding.
            out.push_str(&encoded);
        } else {
            for chunk in encoded.as_bytes().chunks(wrap) {
                out.push_str(&String::from_utf8_lossy(chunk));
                out.push('\n');
            }
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `date [-u] [-d @SECONDS] [+FORMAT]`. Only the `@SECONDS` form of `-d` is
    /// supported. Without `-d`, it reports the current time.
    pub(crate) fn date(&self, args: &[String]) -> ExecutionResult {
        let mut at: Option<i64> = None;
        let mut format: Option<String> = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-u" {
                // The sandbox has one time zone, UTC, so -u changes nothing.
            } else if arg == "-d" {
                match iter
                    .next()
                    .and_then(|v| v.strip_prefix('@'))
                    .and_then(|v| v.parse().ok())
                {
                    Some(secs) => at = Some(secs),
                    None => return failure("date: only -d @SECONDS is supported".to_string()),
                }
            } else if let Some(f) = arg.strip_prefix('+') {
                format = Some(f.to_string());
            } else {
                return failure(format!("date: unsupported argument '{arg}'"));
            }
        }
        let secs = at.unwrap_or_else(|| Utc::now().timestamp());
        let Some(when) = DateTime::<Utc>::from_timestamp(secs, 0) else {
            return failure("date: time out of range".to_string());
        };
        let pattern = format.unwrap_or_else(|| "%a %b %e %H:%M:%S %Z %Y".to_string());
        let rendered = when.format(&pattern).to_string();
        ok(format!("{rendered}\n").into_bytes())
    }
}
