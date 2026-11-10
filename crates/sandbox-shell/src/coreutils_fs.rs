//! Coreutils that walk or describe the VFS: `find`, `xargs`, `du`, `stat`, `realpath`.
//! Each one matches GNU for the options it accepts. Where the VFS cannot match GNU
//! exactly (directory block sizes, permission bits not stored in the tree), the
//! divergence is listed in the PR and in PLAN §14.

use crate::builtins::parse_flags;
use crate::interpreter::{describe, failure, ExecutionResult, Shell};
use sandbox_vfs::SessionFs;
use std::path::Path;

/// Matches a shell glob (`*`, `?`, and literal bytes) against a whole name.
pub(crate) fn glob_match(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|i| glob_match(rest, &name[i..])),
        Some((b'?', rest)) => !name.is_empty() && glob_match(rest, &name[1..]),
        Some((&c, rest)) => name.first() == Some(&c) && glob_match(rest, &name[1..]),
    }
}

/// The last component of a display path, as `find -name` compares it.
fn last_component(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Joins a directory's display path and an entry name, as GNU `find` prints them.
fn join_display(parent: &str, name: &str) -> String {
    if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Splits `xargs` input into arguments on blanks and newlines, honouring single and
/// double quotes. An unmatched quote is an error, as in GNU.
fn xargs_words(input: &[u8]) -> Result<Vec<String>, String> {
    let text = String::from_utf8_lossy(input);
    let mut words = Vec::new();
    let mut current = String::new();
    let mut has_word = false;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                has_word = true;
            }
            None if c.is_whitespace() => {
                if has_word {
                    words.push(std::mem::take(&mut current));
                    has_word = false;
                }
            }
            None => {
                current.push(c);
                has_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("xargs: unmatched quote".to_string());
    }
    if has_word {
        words.push(current);
    }
    Ok(words)
}

impl Shell {
    /// `find PATH... [-name PATTERN] [-type f|d] [-maxdepth N]`. Prints each match.
    /// Entries are visited in the VFS's sorted order, which GNU does not promise.
    pub(crate) async fn find(&self, args: &[String]) -> ExecutionResult {
        let mut starts: Vec<String> = Vec::new();
        let mut name: Option<String> = None;
        let mut kind: Option<char> = None;
        let mut max_depth: Option<usize> = None;
        let mut iter = args.iter();
        let mut in_operands = true;
        while let Some(arg) = iter.next() {
            if in_operands && !arg.starts_with('-') {
                starts.push(arg.clone());
                continue;
            }
            in_operands = false;
            match arg.as_str() {
                "-name" => match iter.next() {
                    Some(v) => name = Some(v.clone()),
                    None => return failure("find: missing argument to '-name'".to_string()),
                },
                "-type" => match iter.next().map(|v| v.as_str()) {
                    Some("f") => kind = Some('f'),
                    Some("d") => kind = Some('d'),
                    _ => return failure("find: unsupported -type (use f or d)".to_string()),
                },
                "-maxdepth" => match iter.next().and_then(|v| v.parse().ok()) {
                    Some(n) => max_depth = Some(n),
                    None => return failure("find: invalid -maxdepth".to_string()),
                },
                "-print" => {}
                other => return failure(format!("find: unsupported predicate '{other}'")),
            }
        }
        if starts.is_empty() {
            starts.push(".".to_string());
        }
        let mut out = String::new();
        let mut errors = Vec::new();
        for start in starts {
            let resolved = self.resolve(&start);
            let meta = match self.fs().stat(Path::new(&resolved)).await {
                Ok(m) => m,
                Err(e) => {
                    errors.push(format!("find: '{start}': {}", describe(&e)));
                    continue;
                }
            };
            // Each stack entry: display path, its resolved path, whether it is a directory,
            // and its depth below the start.
            let mut stack = vec![(start.clone(), resolved, meta.is_dir, 0usize)];
            while let Some((display, resolved, is_dir, depth)) = stack.pop() {
                let matches_name = name
                    .as_deref()
                    .is_none_or(|p| glob_match(p.as_bytes(), last_component(&display).as_bytes()));
                let matches_kind = match kind {
                    None => true,
                    Some('d') => is_dir,
                    Some(_) => !is_dir,
                };
                if matches_name && matches_kind {
                    out.push_str(&display);
                    out.push('\n');
                }
                if !is_dir || max_depth.is_some_and(|m| depth >= m) {
                    continue;
                }
                let entries = match self.fs().readdir(Path::new(&resolved)).await {
                    Ok(e) => e,
                    Err(e) => {
                        errors.push(format!("find: '{display}': {}", describe(&e)));
                        continue;
                    }
                };
                // Pushed in reverse so the first entry is visited first.
                for entry in entries.into_iter().rev() {
                    let child_display = join_display(&display, &entry.name);
                    let child_resolved = join_display(&resolved, &entry.name);
                    stack.push((child_display, child_resolved, entry.is_dir, depth + 1));
                }
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: errors.join("\n").into_bytes(),
        }
    }

    /// `xargs [-n N] COMMAND [ARG...]`: runs COMMAND with the words read from stdin.
    pub(crate) async fn xargs(&mut self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut per_run: Option<usize> = None;
        let mut iter = args.iter();
        let mut command: Vec<String> = Vec::new();
        while let Some(arg) = iter.next() {
            if command.is_empty() && arg == "-n" {
                per_run = match iter.next().and_then(|v| v.parse().ok()) {
                    Some(n) if n > 0 => Some(n),
                    _ => return failure("xargs: invalid -n argument".to_string()),
                };
            } else {
                command.push(arg.clone());
                command.extend(iter.by_ref().cloned());
            }
        }
        if command.is_empty() {
            command.push("echo".to_string());
        }
        let words = match xargs_words(&stdin) {
            Ok(w) => w,
            Err(e) => return failure(e),
        };
        let chunk = per_run.unwrap_or(words.len().max(1));
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut status = 0;
        if words.is_empty() && per_run.is_none() {
            // GNU runs the command once with no arguments when the input is empty.
            return match Box::pin(self.run_argv(&command, &[])).await {
                Ok(r) => r,
                Err(e) => failure(e.message),
            };
        }
        for group in words.chunks(chunk) {
            let mut argv = command.clone();
            argv.extend(group.iter().cloned());
            match Box::pin(self.run_argv(&argv, &[])).await {
                Ok(r) => {
                    out.extend(r.stdout);
                    err.extend(r.stderr);
                    if r.exit_code != 0 {
                        status = 123;
                    }
                }
                Err(e) => return failure(e.message),
            }
        }
        ExecutionResult {
            exit_code: status,
            stdout: out,
            stderr: err,
        }
    }

    /// `du -s [-b] PATH...`: the apparent size of each path, summed over its files.
    /// Directories count as zero bytes. GNU counts their block size, so this differs
    /// from GNU for directories, and matches it for plain files.
    pub(crate) async fn du(&self, args: &[String]) -> ExecutionResult {
        let (flags, operands) = match parse_flags("du", args, "sb") {
            Ok(parsed) => parsed,
            Err(e) => return failure(e),
        };
        if !flags.contains(&'s') {
            return failure("du: only -s is supported".to_string());
        }
        if operands.is_empty() {
            return failure("du: missing operand".to_string());
        }
        let mut out = String::new();
        let mut errors = Vec::new();
        for operand in operands {
            match self.size_of(&self.resolve(operand)).await {
                Ok(total) => out.push_str(&format!("{total}\t{operand}\n")),
                Err(e) => errors.push(format!("du: cannot access '{operand}': {}", describe(&e))),
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: errors.join("\n").into_bytes(),
        }
    }

    /// Sums the apparent sizes under `resolved`, counting files only.
    async fn size_of(&self, resolved: &str) -> Result<u64, sandbox_vfs::VfsError> {
        let meta = self.fs().stat(Path::new(resolved)).await?;
        if !meta.is_dir {
            return Ok(meta.size);
        }
        let mut total = 0;
        let mut stack = vec![resolved.to_string()];
        while let Some(dir) = stack.pop() {
            for entry in self.fs().readdir(Path::new(&dir)).await? {
                let child = join_display(&dir, &entry.name);
                if entry.is_dir {
                    stack.push(child);
                } else {
                    total += entry.size;
                }
            }
        }
        Ok(total)
    }

    /// `stat -c FORMAT PATH...`: prints fields for each path. Supported directives are
    /// `%n` (name), `%s` (size), `%F` (type), and `%a` (octal permissions).
    pub(crate) async fn stat(&self, args: &[String]) -> ExecutionResult {
        let mut format: Option<String> = None;
        let mut operands = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-c" {
                format = iter.next().cloned();
                if format.is_none() {
                    return failure("stat: option requires an argument -- 'c'".to_string());
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("stat: unsupported option '{arg}'"));
            } else {
                operands.push(arg.clone());
            }
        }
        let Some(format) = format else {
            return failure("stat: only -c FORMAT is supported".to_string());
        };
        if operands.is_empty() {
            return failure("stat: missing operand".to_string());
        }
        let mut out = String::new();
        let mut errors = Vec::new();
        for operand in operands {
            match self.fs().stat(Path::new(&self.resolve(&operand))).await {
                Ok(meta) => {
                    let kind = if meta.is_symlink {
                        "symbolic link"
                    } else if meta.is_dir {
                        "directory"
                    } else {
                        "regular file"
                    };
                    let line = format
                        .replace("%n", &operand)
                        .replace("%s", &meta.size.to_string())
                        .replace("%F", kind)
                        .replace("%a", &format!("{:o}", meta.mode & 0o7777));
                    out.push_str(&line);
                    out.push('\n');
                }
                Err(e) => errors.push(format!("stat: cannot stat '{operand}': {}", describe(&e))),
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: errors.join("\n").into_bytes(),
        }
    }

    /// `realpath [--relative-to=DIR] PATH...`: the absolute path of each existing path,
    /// or relative to DIR. Paths that do not exist are an error, as in GNU.
    pub(crate) async fn realpath(&self, args: &[String]) -> ExecutionResult {
        let mut relative_to: Option<String> = None;
        let mut operands = Vec::new();
        for arg in args {
            if let Some(dir) = arg.strip_prefix("--relative-to=") {
                relative_to = Some(self.resolve(dir));
            } else if arg.starts_with('-') && arg.len() > 1 {
                return failure(format!("realpath: unsupported option '{arg}'"));
            } else {
                operands.push(arg.clone());
            }
        }
        if operands.is_empty() {
            return failure("realpath: missing operand".to_string());
        }
        let mut out = String::new();
        let mut errors = Vec::new();
        for operand in operands {
            let resolved = self.resolve(&operand);
            // GNU's default mode requires every component but the last to exist, so the
            // last one may be missing. Its parent must exist.
            let parent = match resolved.rfind('/') {
                Some(0) | None => "/".to_string(),
                Some(index) => resolved[..index].to_string(),
            };
            if let Err(e) = self.fs().stat(Path::new(&parent)).await {
                errors.push(format!("realpath: {operand}: {}", describe(&e)));
                continue;
            }
            let shown = match &relative_to {
                Some(base) => relative_path(base, &resolved),
                None => resolved,
            };
            out.push_str(&shown);
            out.push('\n');
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: errors.join("\n").into_bytes(),
        }
    }
}

/// The path from `base` to `target`, both absolute and normalised.
fn relative_path(base: &str, target: &str) -> String {
    let base_parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    let target_parts: Vec<&str> = target.split('/').filter(|p| !p.is_empty()).collect();
    let common = base_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<&str> = vec![".."; base_parts.len() - common];
    parts.extend(&target_parts[common..]);
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
