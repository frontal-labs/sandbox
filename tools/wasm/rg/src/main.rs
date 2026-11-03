//! `rg`: searches files for a pattern, with the common ripgrep flags.
//!
//! Usage: `rg [-i] [-v] [-n] [-c] [-l] [-o] [-F] [-H|-h] PATTERN [PATH...]`.
//! With no path, stdin is searched. A directory is searched recursively, in
//! sorted order, and entries whose name starts with `.` are skipped. Unlike
//! ripgrep, `.gitignore` is not read.
//!
//! Exit codes follow ripgrep: 0 when a line matched, 1 when nothing matched,
//! 2 on a usage or read error.

use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::Bytes;
use grep_searcher::SearcherBuilder;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Default)]
struct Options {
    ignore_case: bool,
    invert: bool,
    line_numbers: bool,
    count: bool,
    files_with_matches: bool,
    only_matching: bool,
    fixed: bool,
    with_filename: Option<bool>,
    pattern: Option<String>,
    paths: Vec<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut only_positional = false;
    for arg in args {
        if only_positional || !arg.starts_with('-') || arg == "-" {
            if opts.pattern.is_none() {
                opts.pattern = Some(arg);
            } else {
                opts.paths.push(arg);
            }
            continue;
        }
        if arg == "--" {
            only_positional = true;
            continue;
        }
        for flag in arg[1..].chars() {
            match flag {
                'i' => opts.ignore_case = true,
                'v' => opts.invert = true,
                'n' => opts.line_numbers = true,
                'c' => opts.count = true,
                'l' => opts.files_with_matches = true,
                'o' => opts.only_matching = true,
                'F' => opts.fixed = true,
                'H' => opts.with_filename = Some(true),
                'h' => opts.with_filename = Some(false),
                other => return Err(format!("unknown option: -{other}")),
            }
        }
    }
    Ok(opts)
}

/// The files to search, in order. A directory expands to its visible files.
fn expand(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let walk = walkdir::WalkDir::new(path)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
            for entry in walk.filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
                files.push(entry.into_path());
            }
        } else {
            files.push(path.clone());
        }
    }
    files
}

struct Report {
    any_match: bool,
    any_error: bool,
}

fn main() -> ExitCode {
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("rg: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(pattern) = opts.pattern.clone() else {
        eprintln!("usage: rg [-i] [-v] [-n] [-c] [-l] [-o] [-F] [-H|-h] PATTERN [PATH...]");
        return ExitCode::from(2);
    };
    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(opts.ignore_case)
        .fixed_strings(opts.fixed)
        .build(&pattern)
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!("rg: regex parse error: {e}");
            return ExitCode::from(2);
        }
    };

    let targets: Vec<PathBuf> = if opts.paths.is_empty() {
        vec![PathBuf::from("-")]
    } else {
        opts.paths.iter().map(PathBuf::from).collect()
    };
    let files = expand(&targets);
    let with_filename = opts
        .with_filename
        .unwrap_or(files.len() > 1 || targets.iter().any(|t| t.is_dir()));

    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut report = Report { any_match: false, any_error: false };
    for file in &files {
        if let Err(e) = search_one(&matcher, &opts, file, with_filename, &mut out, &mut report) {
            eprintln!("rg: {}: {e}", file.display());
            report.any_error = true;
        }
    }
    if out.flush().is_err() {
        return ExitCode::from(2);
    }
    if report.any_error {
        ExitCode::from(2)
    } else if report.any_match {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Searches one file, or stdin for `-`, and writes its report.
fn search_one(
    matcher: &impl Matcher,
    opts: &Options,
    file: &Path,
    with_filename: bool,
    out: &mut impl Write,
    report: &mut Report,
) -> io::Result<()> {
    let bytes = if file == Path::new("-") {
        let mut buf = Vec::new();
        io::stdin().lock().read_to_end(&mut buf)?;
        buf
    } else {
        std::fs::read(file)?
    };
    let label = if file == Path::new("-") {
        // ripgrep's own label for stdin.
        "<stdin>".to_string()
    } else {
        file.display().to_string()
    };
    let prefix = |line: Option<u64>| {
        let mut p = String::new();
        if with_filename {
            p.push_str(&label);
            p.push(':');
        }
        if let (true, Some(n)) = (opts.line_numbers, line) {
            p.push_str(&format!("{n}:"));
        }
        p
    };

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .invert_match(opts.invert)
        .build();
    let mut count: u64 = 0;
    let mut write_err: Option<io::Error> = None;
    searcher.search_slice(
        matcher,
        &bytes,
        Bytes(|line_no, line| {
            count += 1;
            let text = line.strip_suffix(b"\n").unwrap_or(line);
            let emit = !(opts.count || opts.files_with_matches);
            if emit && write_err.is_none() {
                let result = if opts.only_matching {
                    let mut res = Ok(());
                    let _ = matcher.find_iter(text, |m| {
                        res = writeln!(out, "{}{}", prefix(Some(line_no)), String::from_utf8_lossy(&text[m]));
                        res.is_ok()
                    });
                    res
                } else {
                    out.write_all(prefix(Some(line_no)).as_bytes())
                        .and_then(|_| out.write_all(text))
                        .and_then(|_| out.write_all(b"\n"))
                };
                if let Err(e) = result {
                    write_err = Some(e);
                }
            }
            Ok(true)
        }),
    )?;
    if let Some(e) = write_err {
        return Err(e);
    }
    if count > 0 {
        report.any_match = true;
    }
    // Like ripgrep, a count is printed only for a file that matched.
    if opts.count && count > 0 {
        if with_filename {
            writeln!(out, "{label}:{count}")?;
        } else {
            writeln!(out, "{count}")?;
        }
    }
    if opts.files_with_matches && count > 0 {
        writeln!(out, "{label}")?;
    }
    Ok(())
}
