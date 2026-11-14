//! Commands whose output for each input line depends only on that line, so they can run as
//! live input arrives (ADR 0047). `is_streamable` says which argv qualify. Anything it refuses
//! waits for the whole input, as before. The rule is conservative: a flag that needs a count, a
//! line number, or the whole input is refused, not approximated.

/// How a streamed command turns each input line into output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// The command's own output for one line.
    PerLine,
    /// `head -n N`: the first `N` lines, then stop reading.
    Head { remaining: usize },
    /// `nl`: numbered lines. `all` numbers blank lines too.
    Nl { all: bool, number: u64 },
}

impl Filter {
    /// The filter for a streamable argv.
    pub fn for_argv(argv: &[String]) -> Option<Self> {
        if !is_streamable(argv) {
            return None;
        }
        match argv[0].as_str() {
            "head" => Some(Filter::Head {
                remaining: argv[2].parse().unwrap_or(0),
            }),
            "nl" => Some(Filter::Nl {
                all: argv.len() == 3,
                number: 0,
            }),
            _ => Some(Filter::PerLine),
        }
    }
}

/// Whether the argv can run line by line over live input.
pub fn is_streamable(argv: &[String]) -> bool {
    let Some((name, args)) = argv.split_first() else {
        return false;
    };
    match name.as_str() {
        "cat" | "rev" => args.is_empty(),
        "tr" => args.len() == 2 && !args.iter().any(|a| a.starts_with('-')),
        "grep" => one_operand_with_flags(args, "FiEv"),
        "sed" => one_operand_with_flags(args, "nE"),
        "cut" => cut_streamable(args),
        "head" => matches!(args, [flag, n] if flag == "-n" && n.parse::<usize>().is_ok()),
        "nl" => args.is_empty() || matches!(args, [flag, v] if flag == "-b" && v == "a"),
        _ => false,
    }
}

/// Whether the argv reads standard input when it runs, for a command that needs all of its input
/// (see `Shell::drain_live`). A command given a file operand reads that file, not the input.
/// Commands not listed here keep their earlier behaviour, which takes no standard input.
pub fn reads_standard_input(argv: &[String]) -> bool {
    let Some((name, args)) = argv.split_first() else {
        return false;
    };
    // Options that take a value, so the value is not an operand.
    let operands = || {
        let mut operands = Vec::new();
        let mut skip_value = false;
        for arg in args {
            if skip_value {
                skip_value = false;
            } else if matches!(arg.as_str(), "-n" | "-c" | "-d" | "-f" | "-b" | "-w" | "-s") {
                skip_value = true;
            } else if !arg.starts_with('-') || arg == "-" {
                operands.push(arg.as_str());
            }
        }
        operands
    };
    match name.as_str() {
        // Always read standard input.
        "tr" | "tee" | "xargs" => true,
        // Read standard input when no file is named.
        "cat" | "wc" | "sort" | "uniq" | "head" | "tail" | "rev" | "nl" | "base64" | "paste"
        | "cut" => operands().is_empty(),
        // The first operand is the pattern or script; the rest name files.
        "grep" | "sed" => operands().len() <= 1,
        _ => false,
    }
}

/// Flags from `allowed` and exactly one operand. Any other flag refuses.
fn one_operand_with_flags(args: &[String], allowed: &str) -> bool {
    let mut operands = 0;
    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            if !arg[1..].chars().all(|c| allowed.contains(c)) {
                return false;
            }
        } else {
            operands += 1;
        }
    }
    operands == 1
}

/// `cut` with `-d`, `-f`, `-c`, and `-s`, and no file operand. It needs a field or character list.
fn cut_streamable(args: &[String]) -> bool {
    let mut has_list = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-s" => {}
            "-d" | "-f" | "-c" => {
                let Some(value) = iter.next() else {
                    return false;
                };
                has_list |= arg != "-d" && !value.is_empty();
            }
            other => {
                if let Some(rest) = other
                    .strip_prefix("-d")
                    .or(other.strip_prefix("-f"))
                    .or(other.strip_prefix("-c"))
                {
                    has_list |= other.starts_with("-f") || other.starts_with("-c");
                    if rest.is_empty() {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
    }
    has_list
}
