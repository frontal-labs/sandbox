//! Commands whose output for each input line depends only on that line, so they can run as
//! live input arrives (ADR 0047). `is_streamable` says which argv qualify. Anything it refuses
//! waits for the whole input, as before. The rule is conservative: a flag that needs a count, a
//! line number, or the whole input is refused, not approximated.

/// How a streamed command turns each input line into output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Filter {
    /// The command's own output for one line.
    PerLine,
    /// `head -n N`: the first `N` lines, then stop reading.
    Head { remaining: usize },
    /// `nl`: numbered lines. `all` numbers blank lines too.
    Nl { all: bool, number: u64 },
}

impl Filter {
    /// The filter for a streamable argv.
    pub(crate) fn for_argv(argv: &[String]) -> Option<Self> {
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
pub(crate) fn is_streamable(argv: &[String]) -> bool {
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
pub(crate) fn reads_standard_input(argv: &[String]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn line_local_commands_stream() {
        assert!(is_streamable(&argv(&["cat"])));
        assert!(is_streamable(&argv(&["rev"])));
        assert!(is_streamable(&argv(&["tr", "a-z", "A-Z"])));
        assert!(is_streamable(&argv(&["grep", "-v", "x"])));
        assert!(is_streamable(&argv(&["sed", "-n", "s/a/b/p"])));
        assert!(is_streamable(&argv(&["cut", "-d:", "-f2"])));
        assert!(is_streamable(&argv(&["cut", "-d", ":", "-f", "2"])));
        assert!(is_streamable(&argv(&["head", "-n", "2"])));
        assert!(is_streamable(&argv(&["nl"])));
        assert!(is_streamable(&argv(&["nl", "-b", "a"])));
    }

    #[test]
    fn commands_that_need_the_whole_input_or_a_file_do_not_stream() {
        assert!(!is_streamable(&argv(&["sort"])));
        assert!(!is_streamable(&argv(&["wc", "-l"])));
        assert!(!is_streamable(&argv(&["tail", "-n", "2"])));
        assert!(!is_streamable(&argv(&["cat", "file"])));
        assert!(
            !is_streamable(&argv(&["grep", "-c", "x"])),
            "a count needs every line"
        );
        assert!(
            !is_streamable(&argv(&["grep", "-n", "x"])),
            "a line number needs every line"
        );
        assert!(!is_streamable(&argv(&["grep", "x", "file"])));
        assert!(!is_streamable(&argv(&["tr", "-s", "a"])));
        assert!(!is_streamable(&argv(&["cut", "file"])), "cut needs a list");
        assert!(!is_streamable(&argv(&["head"])));
        assert!(!is_streamable(&argv(&["nl", "-v", "2"])));
    }

    #[test]
    fn filters_start_with_their_counters() {
        assert_eq!(
            Filter::for_argv(&argv(&["head", "-n", "3"])),
            Some(Filter::Head { remaining: 3 })
        );
        assert_eq!(
            Filter::for_argv(&argv(&["nl"])),
            Some(Filter::Nl {
                all: false,
                number: 0
            })
        );
        assert_eq!(
            Filter::for_argv(&argv(&["nl", "-b", "a"])),
            Some(Filter::Nl {
                all: true,
                number: 0
            })
        );
        assert_eq!(Filter::for_argv(&argv(&["rev"])), Some(Filter::PerLine));
        assert_eq!(Filter::for_argv(&argv(&["sort"])), None);
    }

    #[test]
    fn a_command_reads_standard_input_only_when_no_file_is_named() {
        assert!(reads_standard_input(&argv(&["sort"])));
        assert!(reads_standard_input(&argv(&["sort", "-r"])));
        assert!(reads_standard_input(&argv(&["cat"])));
        assert!(!reads_standard_input(&argv(&["cat", "file"])));
        assert!(
            reads_standard_input(&argv(&["head", "-n", "2"])),
            "the value of -n is not a file"
        );
        assert!(reads_standard_input(&argv(&["grep", "pattern"])));
        assert!(!reads_standard_input(&argv(&["grep", "pattern", "file"])));
        assert!(reads_standard_input(&argv(&["tr", "a", "b"])));
        assert!(
            reads_standard_input(&argv(&["tee", "out"])),
            "tee always reads its input"
        );
        assert!(!reads_standard_input(&argv(&["echo", "hi"])));
        assert!(
            !reads_standard_input(&argv(&["node", "script.js"])),
            "external tools keep their behaviour"
        );
    }
}
