//! Coreutils builtins that run against the VFS (PLAN §4.4). Each command follows GNU
//! behaviour for the options it accepts, and rejects the options it does not, so a
//! script never gets silently different output. `conformance/coreutils.json` holds the
//! GNU output for each supported option, recorded by `record.py`.

use crate::builtins::{ok, parse_flags, Inputs};
use crate::interpreter::{describe, failure, failure_with, ExecutionResult, Shell};
use std::cmp::Ordering;

/// Splits data into lines without their newlines. A final line without a newline is a
/// line, and an empty input has no lines, as in GNU.
fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    let body = data.strip_suffix(b"\n").unwrap_or(data);
    body.split(|&b| b == b'\n').collect()
}

/// Appends `line` and a newline to `out`.
fn push_line(out: &mut Vec<u8>, line: &[u8]) {
    out.extend_from_slice(line);
    out.push(b'\n');
}

/// The leading number of a line for `sort -n`, as GNU reads it: blanks, an optional
/// minus sign, digits, and an optional fraction. Anything else is zero.
fn numeric_key(line: &[u8]) -> f64 {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    if bytes.first() == Some(&b'-') {
        end = 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let mut has_digits = end > digits_start;
    if end < bytes.len() && bytes[end] == b'.' {
        let frac_start = end + 1;
        let mut frac_end = frac_start;
        while frac_end < bytes.len() && bytes[frac_end].is_ascii_digit() {
            frac_end += 1;
        }
        has_digits |= frac_end > frac_start;
        end = frac_end;
    }
    if !has_digits {
        return 0.0;
    }
    trimmed[..end].parse().unwrap_or(0.0)
}

/// Parses a `cut` list such as `1,3-5,-2,4-`. Each entry is a 1-based position or a
/// range, and an open end runs to the end of the line. Returns `None` if the list is
/// malformed.
fn parse_cut_list(list: &str) -> Option<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();
    for item in list.split(',') {
        let (start, end) = match item.split_once('-') {
            None => {
                let n: usize = item.parse().ok()?;
                (n, n)
            }
            Some((a, b)) => {
                let start = if a.is_empty() { 1 } else { a.parse().ok()? };
                let end = if b.is_empty() {
                    usize::MAX
                } else {
                    b.parse().ok()?
                };
                (start, end)
            }
        };
        if start == 0 || (end != usize::MAX && end < start) {
            return None;
        }
        ranges.push((start, end));
    }
    Some(ranges)
}

fn in_ranges(ranges: &[(usize, usize)], position: usize) -> bool {
    ranges
        .iter()
        .any(|&(lo, hi)| position >= lo && position <= hi)
}

/// Expands a `tr` set: escapes `\n \t \\`, and `a-z` ranges.
fn expand_tr_set(set: &str) -> Result<Vec<u8>, String> {
    let raw = set.as_bytes();
    let mut chars = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let byte = if raw[i] == b'\\' && i + 1 < raw.len() {
            i += 1;
            match raw[i] {
                b'n' => b'\n',
                b't' => b'\t',
                b'\\' => b'\\',
                other => other,
            }
        } else if raw[i] == b'[' && raw[i..].starts_with(b"[:") {
            return Err("tr: character classes are not supported".to_string());
        } else {
            raw[i]
        };
        i += 1;
        // A range `a-z`, where the dash is between two characters.
        if i + 1 < raw.len() && raw[i] == b'-' {
            let last = raw[i + 1];
            if last < byte {
                return Err(format!(
                    "tr: range-endpoints of '{}-{}' are in reverse collating sequence order",
                    byte as char, last as char
                ));
            }
            chars.extend(byte..=last);
            i += 2;
        } else {
            chars.push(byte);
        }
    }
    Ok(chars)
}

/// The value of an option given as `-dX` (inline) or `-d X` (the next argument).
fn value_of<'a>(inline: &str, rest: &mut impl Iterator<Item = &'a String>) -> Option<String> {
    if inline.is_empty() {
        rest.next().cloned()
    } else {
        Some(inline.to_string())
    }
}

impl Shell {
    /// `sort [-n] [-r] [-u]`: sorts lines bytewise, or by their leading number with `-n`.
    pub(crate) async fn sort(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("sort", args, "nru") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        let operands: Vec<String> = operands.iter().map(|s| s.to_string()).collect();
        let inputs = self.read_inputs("sort", &operands, stdin).await;
        let numeric = flags.contains(&'n');
        let reverse = flags.contains(&'r');
        let unique = flags.contains(&'u');
        let mut lines: Vec<&[u8]> = inputs.data.iter().flat_map(|d| split_lines(d)).collect();
        // GNU drops the whole-line tie-break under -u, so lines equal by key keep their
        // input order and the first one wins below. The sort is stable, so that holds.
        let compare = |a: &&[u8], b: &&[u8]| -> Ordering {
            let ord = if numeric {
                let by_key = numeric_key(a)
                    .partial_cmp(&numeric_key(b))
                    .unwrap_or(Ordering::Equal);
                if unique {
                    by_key
                } else {
                    by_key.then_with(|| a.cmp(b))
                }
            } else {
                a.cmp(b)
            };
            if reverse {
                ord.reverse()
            } else {
                ord
            }
        };
        lines.sort_by(compare);
        if unique {
            // With -u, lines that compare equal by key are one line, and the first is kept.
            let same_key = |a: &&[u8], b: &&[u8]| -> bool {
                if numeric {
                    numeric_key(a) == numeric_key(b)
                } else {
                    a == b
                }
            };
            lines.dedup_by(|b, a| same_key(a, b));
        }
        let mut out = Vec::new();
        for line in lines {
            push_line(&mut out, line);
        }
        ExecutionResult {
            // GNU sort reports trouble, such as an unreadable file, with status 2.
            exit_code: if inputs.errors.is_empty() { 0 } else { 2 },
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `uniq [-c] [-d] [-u]`: collapses adjacent equal lines.
    pub(crate) async fn uniq(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("uniq", args, "cdu") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        if operands.len() > 1 {
            return failure("uniq: an output file operand is not supported".to_string());
        }
        let operands: Vec<String> = operands.iter().map(|s| s.to_string()).collect();
        let inputs = self.read_inputs("uniq", &operands, stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        let lines = split_lines(&data);
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j] == lines[i] {
                j += 1;
            }
            let count = j - i;
            let keep = match (flags.contains(&'d'), flags.contains(&'u')) {
                (true, _) => count > 1,
                (false, true) => count == 1,
                (false, false) => true,
            };
            if keep {
                if flags.contains(&'c') {
                    out.extend_from_slice(format!("{count:>7} ").as_bytes());
                }
                push_line(&mut out, lines[i]);
            }
            i = j;
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `cut -d DELIM -f LIST | -c LIST [-s]`: selects fields or characters from each line.
    pub(crate) async fn cut(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut delimiter = b'\t';
        let mut fields: Option<String> = None;
        let mut chars: Option<String> = None;
        let mut only_delimited = false;
        let mut operands = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if let Some(rest) = arg.strip_prefix("-d") {
                let value = match value_of(rest, &mut iter) {
                    Some(v) => v,
                    None => return failure("cut: option requires an argument -- 'd'".to_string()),
                };
                if value.len() != 1 {
                    return failure("cut: the delimiter must be a single character".to_string());
                }
                delimiter = value.as_bytes()[0];
            } else if let Some(rest) = arg.strip_prefix("-f") {
                fields = match value_of(rest, &mut iter) {
                    Some(v) => Some(v),
                    None => return failure("cut: option requires an argument -- 'f'".to_string()),
                };
            } else if let Some(rest) = arg.strip_prefix("-c") {
                chars = match value_of(rest, &mut iter) {
                    Some(v) => Some(v),
                    None => return failure("cut: option requires an argument -- 'c'".to_string()),
                };
            } else if arg == "-s" {
                only_delimited = true;
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("cut: invalid option -- '{}'", &arg[1..]));
            } else {
                operands.push(arg.clone());
            }
        }
        let (list, by_field) = match (fields, chars) {
            (Some(f), None) => (f, true),
            (None, Some(c)) => (c, false),
            _ => {
                return failure(
                    "cut: you must specify a list of bytes, characters, or fields".to_string(),
                )
            }
        };
        let Some(ranges) = parse_cut_list(&list) else {
            return failure(format!("cut: invalid field or byte range: '{list}'"));
        };
        let inputs = self.read_inputs("cut", &operands, stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        let mut out = Vec::new();
        for line in split_lines(&data) {
            if !by_field {
                let selected: Vec<u8> = line
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| in_ranges(&ranges, i + 1))
                    .map(|(_, &b)| b)
                    .collect();
                push_line(&mut out, &selected);
                continue;
            }
            if !line.contains(&delimiter) {
                if !only_delimited {
                    push_line(&mut out, line);
                }
                continue;
            }
            let picked: Vec<&[u8]> = line
                .split(|&b| b == delimiter)
                .enumerate()
                .filter(|(i, _)| in_ranges(&ranges, i + 1))
                .map(|(_, f)| f)
                .collect();
            let joined = picked.join(&delimiter);
            push_line(&mut out, &joined);
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `tr [-d] [-s] SET1 [SET2]`: translates, deletes, or squeezes bytes of stdin.
    pub(crate) fn tr(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("tr", args, "ds") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        let delete = flags.contains(&'d');
        let squeeze = flags.contains(&'s');
        let set1 = match operands.first() {
            Some(s) => match expand_tr_set(s) {
                Ok(set) => set,
                Err(e) => return failure(e),
            },
            None => return failure("tr: missing operand".to_string()),
        };
        let set2 = match operands.get(1) {
            Some(s) => match expand_tr_set(s) {
                Ok(set) => Some(set),
                Err(e) => return failure(e),
            },
            None => None,
        };
        if operands.len() > 2 {
            return failure(format!("tr: extra operand '{}'", operands[2]));
        }
        if delete {
            let mut out = Vec::new();
            for b in stdin {
                if !set1.contains(&b) {
                    out.push(b);
                }
            }
            return ok(out);
        }
        // Without -d, SET1 translates to SET2. A short SET2 repeats its last byte.
        let mut map: [Option<u8>; 256] = [None; 256];
        if let Some(set2) = set2.as_ref() {
            if set2.is_empty() {
                return failure("tr: when not deleting, SET2 must not be empty".to_string());
            }
            for (i, &from) in set1.iter().enumerate() {
                let to = set2[i.min(set2.len() - 1)];
                map[from as usize] = Some(to);
            }
        }
        let squeeze_set: Vec<u8> = if squeeze {
            match set2.as_ref() {
                Some(set2) => set2.clone(),
                None => set1.clone(),
            }
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(stdin.len());
        let mut previous: Option<u8> = None;
        for b in stdin {
            let translated = map[b as usize].unwrap_or(b);
            if squeeze && previous == Some(translated) && squeeze_set.contains(&translated) {
                continue;
            }
            out.push(translated);
            previous = Some(translated);
        }
        ok(out)
    }

    /// `tee [-a] [FILE...]`: copies stdin to stdout and to each file.
    pub(crate) async fn tee(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("tee", args, "a") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        let append = flags.contains(&'a');
        let mut errors = Vec::new();
        for operand in operands {
            let resolved = self.resolve(operand);
            let mut data = Vec::new();
            if append {
                if let Ok(existing) = self.read_file(operand).await {
                    data = existing;
                }
            }
            data.extend_from_slice(&stdin);
            if let Err(e) = self.write_resolved(&resolved, &data).await {
                errors.push(format!("tee: {operand}: {}", describe(&e)));
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: stdin,
            stderr: errors.join("\n").into_bytes(),
        }
    }

    /// `seq [-s SEP] [-w] [FIRST [INCREMENT]] LAST` for integers.
    pub(crate) fn seq(&self, args: &[String]) -> ExecutionResult {
        let mut separator = "\n".to_string();
        let mut equal_width = false;
        let mut numbers: Vec<String> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-s" {
                match iter.next() {
                    Some(v) => separator = v.clone(),
                    None => return failure("seq: option requires an argument -- 's'".to_string()),
                }
            } else if let Some(v) = arg.strip_prefix("-s") {
                separator = v.to_string();
            } else if arg == "-w" {
                equal_width = true;
            } else {
                numbers.push(arg.clone());
            }
        }
        let parse = |s: &str| -> Result<i64, String> {
            s.parse::<i64>()
                .map_err(|_| format!("seq: invalid floating point argument: '{s}'"))
        };
        let (first, step, last) = match numbers.as_slice() {
            [last] => (Ok(1), Ok(1), parse(last)),
            [first, last] => (parse(first), Ok(1), parse(last)),
            [first, step, last] => (parse(first), parse(step), parse(last)),
            _ => return failure("seq: missing operand".to_string()),
        };
        let (first, step, last) = match (first, step, last) {
            (Ok(a), Ok(b), Ok(c)) => (a, b, c),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return failure(e),
        };
        if step == 0 {
            return failure("seq: zero increment".to_string());
        }
        let mut values = Vec::new();
        let mut current = first;
        while (step > 0 && current <= last) || (step < 0 && current >= last) {
            values.push(current);
            current += step;
        }
        let width = if equal_width {
            values
                .iter()
                .map(|v| v.to_string().len())
                .chain(std::iter::once(first.to_string().len()))
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        let rendered: Vec<String> = values
            .iter()
            .map(|v| {
                if equal_width {
                    if *v < 0 {
                        format!("-{:0>w$}", v.unsigned_abs(), w = width - 1)
                    } else {
                        format!("{v:0>width$}")
                    }
                } else {
                    v.to_string()
                }
            })
            .collect();
        if rendered.is_empty() {
            return ok(Vec::new());
        }
        ok(format!("{}\n", rendered.join(&separator)).into_bytes())
    }

    /// `paste [-d LIST] [-s] [FILE...]`: joins corresponding lines, or with `-s`, each file.
    pub(crate) async fn paste(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut delimiters: Vec<u8> = vec![b'\t'];
        let mut serial = false;
        let mut operands = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-s" {
                serial = true;
            } else if let Some(rest) = arg.strip_prefix("-d") {
                let value = if rest.is_empty() {
                    match iter.next() {
                        Some(v) => v.clone(),
                        None => {
                            return failure("paste: option requires an argument -- 'd'".to_string())
                        }
                    }
                } else {
                    rest.to_string()
                };
                delimiters = value.into_bytes();
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("paste: invalid option -- '{}'", &arg[1..]));
            } else {
                operands.push(arg.clone());
            }
        }
        let Inputs { data, errors } = self.read_inputs("paste", &operands, stdin).await;
        let files: Vec<Vec<&[u8]>> = data.iter().map(|d| split_lines(d)).collect();
        let mut out = Vec::new();
        if serial {
            for lines in &files {
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        out.push(delimiters[(i - 1) % delimiters.len()]);
                    }
                    out.extend_from_slice(line);
                }
                out.push(b'\n');
            }
        } else {
            let rows = files.iter().map(|f| f.len()).max().unwrap_or(0);
            for row in 0..rows {
                for (col, lines) in files.iter().enumerate() {
                    if col > 0 {
                        out.push(delimiters[(col - 1) % delimiters.len()]);
                    }
                    if let Some(line) = lines.get(row) {
                        out.extend_from_slice(line);
                    }
                }
                out.push(b'\n');
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out,
            stderr: errors.join("\n").into_bytes(),
        }
    }

    /// `rev`: reverses the bytes of each line.
    pub(crate) async fn rev(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let inputs = self.read_inputs("rev", args, stdin).await;
        let data: Vec<u8> = inputs.data.concat();
        let mut out = Vec::new();
        for line in split_lines(&data) {
            let reversed: Vec<u8> = line.iter().rev().copied().collect();
            push_line(&mut out, &reversed);
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// `basename NAME [SUFFIX]` and `basename -s SUFFIX NAME...`, `basename -a NAME...`.
    pub(crate) fn basename(&self, args: &[String]) -> ExecutionResult {
        let mut multiple = false;
        let mut suffix: Option<String> = None;
        let mut names = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-a" {
                multiple = true;
            } else if arg == "-s" {
                multiple = true;
                match iter.next() {
                    Some(v) => suffix = Some(v.clone()),
                    None => {
                        return failure("basename: option requires an argument -- 's'".to_string())
                    }
                }
            } else {
                names.push(arg.clone());
            }
        }
        if names.is_empty() {
            return failure("basename: missing operand".to_string());
        }
        let (paths, suffix) = if multiple {
            (names, suffix)
        } else {
            let suffix = names.get(1).cloned();
            (names.into_iter().take(1).collect(), suffix)
        };
        let mut out = Vec::new();
        for path in paths {
            let mut base = basename_of(&path).to_string();
            if let Some(sfx) = suffix.as_deref() {
                if base.len() > sfx.len() && base.ends_with(sfx) {
                    base.truncate(base.len() - sfx.len());
                }
            }
            push_line(&mut out, base.as_bytes());
        }
        ok(out)
    }

    /// `dirname NAME...`: the directory part of each name.
    pub(crate) fn dirname(&self, args: &[String]) -> ExecutionResult {
        if args.is_empty() {
            return failure("dirname: missing operand".to_string());
        }
        let mut out = Vec::new();
        for path in args {
            push_line(&mut out, dirname_of(path).as_bytes());
        }
        ok(out)
    }

    /// `printenv [NAME...]`: prints the named variables, or all of them.
    pub(crate) fn printenv(&self, args: &[String]) -> ExecutionResult {
        if args.is_empty() {
            let mut names: Vec<&String> = self.env().keys().collect();
            names.sort();
            let mut out = Vec::new();
            for name in names {
                out.extend_from_slice(format!("{name}={}\n", self.env()[name]).as_bytes());
            }
            return ok(out);
        }
        let mut out = Vec::new();
        let mut status = 0;
        for name in args {
            match self.env().get(name) {
                Some(value) => out.extend_from_slice(format!("{value}\n").as_bytes()),
                None => status = 1,
            }
        }
        ExecutionResult {
            exit_code: status,
            stdout: out,
            stderr: Vec::new(),
        }
    }

    /// `cmp [-s] FILE1 FILE2`: reports the first byte that differs, or nothing when the
    /// files are equal. Exit status is 0 for equal, 1 for different, and 2 on error.
    pub(crate) async fn cmp(&self, args: &[String]) -> ExecutionResult {
        let (flags, operands) = match parse_flags("cmp", args, "s") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        let silent = flags.contains(&'s');
        if operands.len() != 2 {
            return failure("cmp: two file operands are required".to_string());
        }
        let (a_name, b_name) = (operands[0], operands[1]);
        let a = match self.read_file(a_name).await {
            Ok(d) => d,
            Err(e) => return failure_with(format!("cmp: {a_name}: {}", describe(&e)), 2),
        };
        let b = match self.read_file(b_name).await {
            Ok(d) => d,
            Err(e) => return failure_with(format!("cmp: {b_name}: {}", describe(&e)), 2),
        };
        let common = a.len().min(b.len());
        let first_diff = (0..common).find(|&i| a[i] != b[i]);
        match first_diff {
            Some(index) => {
                if silent {
                    return failure_with(String::new(), 1);
                }
                let line = a[..=index].iter().filter(|&&c| c == b'\n').count() + 1;
                ok(format!(
                    "{a_name} {b_name} differ: char {}, line {line}\n",
                    index + 1
                )
                .into_bytes())
                .with_exit(1)
            }
            None if a.len() == b.len() => ok(Vec::new()),
            None => {
                if silent {
                    return failure_with(String::new(), 1);
                }
                let (shorter, after) = if a.len() < b.len() {
                    (a_name, common)
                } else {
                    (b_name, common)
                };
                failure_with(format!("cmp: EOF on {shorter} after byte {after}"), 1)
            }
        }
    }
}

trait WithExit {
    fn with_exit(self, code: i32) -> Self;
}

impl WithExit for ExecutionResult {
    fn with_exit(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }
}

/// The last component of a path, with trailing slashes removed.
fn basename_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The directory part of a path, as GNU `dirname` reports it.
fn dirname_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() {
            ".".to_string()
        } else {
            "/".to_string()
        };
    }
    match trimmed.rfind('/') {
        None => ".".to_string(),
        Some(index) => {
            let head = trimmed[..index].trim_end_matches('/');
            if head.is_empty() {
                "/".to_string()
            } else {
                head.to_string()
            }
        }
    }
}
