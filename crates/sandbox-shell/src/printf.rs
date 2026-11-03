//! The `printf` formatter.
//!
//! The format is processed as bash processes it. Escapes such as `\n` are
//! expanded in the format. The directives are `%s`, `%b`, `%d`, `%i`, `%u`,
//! `%x`, `%X`, `%o`, `%c`, and `%%`, with the flags `-`, `0`, `+`, and a width.
//! When there are more arguments than directives, the format is reused. A
//! missing argument counts as empty, or as 0 for a number.
//!
//! Floating point directives are not supported. They are an error, not a
//! silent difference.

/// The result of formatting: the output, and the first error that was found.
/// As in bash, an argument that is not a number is reported and formats as 0,
/// and the rest of the output is still produced.
#[derive(Debug)]
pub struct Formatted {
    pub output: Vec<u8>,
    pub error: Option<String>,
}

/// Formats `args` with `format`. An unsupported directive stops the format, and
/// the error is returned with no output.
pub fn format(format: &str, args: &[String]) -> Result<Formatted, String> {
    let mut out = Vec::new();
    let mut error = None;
    let mut next_arg = 0;
    loop {
        let consumed = render_once(format, args, &mut next_arg, &mut out, &mut error)?;
        // Reuse the format while arguments remain, but stop after one pass that
        // used none, so a format without directives prints once.
        if next_arg >= args.len() || !consumed {
            return Ok(Formatted { output: out, error });
        }
    }
}

/// One pass over the format. Returns whether any argument was used.
fn render_once(
    format: &str,
    args: &[String],
    next_arg: &mut usize,
    out: &mut Vec<u8>,
    error: &mut Option<String>,
) -> Result<bool, String> {
    let chars: Vec<char> = format.chars().collect();
    let mut used = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let (bytes, advance) = escape(&chars[i..]);
                out.extend(bytes);
                i += advance;
            }
            '%' => {
                i += 1;
                if chars.get(i) == Some(&'%') {
                    out.push(b'%');
                    i += 1;
                } else {
                    let (spec, advance) = read_spec(&chars[i..])?;
                    i += advance;
                    let arg = args.get(*next_arg).cloned();
                    if arg.is_some() {
                        used = true;
                    }
                    *next_arg += usize::from(arg.is_some());
                    let (bytes, problem) = render(&spec, arg.as_deref());
                    out.extend(bytes);
                    if error.is_none() {
                        *error = problem;
                    }
                }
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                i += 1;
            }
        }
    }
    Ok(used)
}

/// A parsed directive: flags, width, and conversion.
struct Spec {
    left: bool,
    zero: bool,
    plus: bool,
    width: usize,
    conversion: char,
}

/// Reads `[flags][width]conversion` after a `%`. Returns the spec and how many
/// characters it used.
fn read_spec(chars: &[char]) -> Result<(Spec, usize), String> {
    // Flags are the run of `-`, `0`, and `+` at the start.
    let flags_end = chars
        .iter()
        .position(|c| !matches!(c, '-' | '0' | '+'))
        .unwrap_or(chars.len());
    let flags = &chars[..flags_end];
    let (left, zero, plus) = (
        flags.contains(&'-'),
        flags.contains(&'0'),
        flags.contains(&'+'),
    );
    let mut i = flags_end;
    let mut width = 0usize;
    while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
        width = width * 10 + d as usize;
        i += 1;
    }
    let conversion = *chars.get(i).ok_or("printf: missing format character")?;
    if !matches!(
        conversion,
        's' | 'b' | 'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'c'
    ) {
        return Err(format!("printf: %{conversion}: unsupported format"));
    }
    Ok((
        Spec {
            left,
            zero,
            plus,
            width,
            conversion,
        },
        i + 1,
    ))
}

/// Renders one directive with its argument. Also returns a problem with the
/// argument, if it has one. The directive still renders, with the value 0.
fn render(spec: &Spec, arg: Option<&str>) -> (Vec<u8>, Option<String>) {
    let numeric = matches!(spec.conversion, 'd' | 'i' | 'u' | 'x' | 'X' | 'o');
    let (value, problem) = if numeric { number(arg) } else { (0, None) };
    let bytes = match spec.conversion {
        's' => pad_text(spec, arg.unwrap_or("")),
        'b' => pad_text(spec, &expand_escapes(arg.unwrap_or(""))),
        'c' => pad_text(
            spec,
            &arg.and_then(|a| a.chars().next())
                .map(String::from)
                .unwrap_or_default(),
        ),
        'd' | 'i' => {
            let sign = if value >= 0 && spec.plus { "+" } else { "" };
            pad_number(spec, &format!("{sign}{value}"))
        }
        'u' => pad_number(spec, &(value as u64).to_string()),
        'x' => pad_number(spec, &format!("{:x}", value as u64)),
        'X' => pad_number(spec, &format!("{:X}", value as u64)),
        'o' => pad_number(spec, &format!("{:o}", value as u64)),
        _ => unreachable!("read_spec only returns the conversions above"),
    };
    (bytes, problem)
}

/// The integer value of an argument. An empty or missing one is 0. Anything
/// else that is not an integer is 0 with a problem, as in bash.
fn number(arg: Option<&str>) -> (i64, Option<String>) {
    let text = arg.unwrap_or("").trim();
    if text.is_empty() {
        return (0, None);
    }
    match text.parse::<i64>() {
        Ok(value) => (value, None),
        Err(_) => (0, Some(format!("printf: {text}: invalid number"))),
    }
}

fn pad_text(spec: &Spec, text: &str) -> Vec<u8> {
    let len = text.chars().count();
    let padding = spec.width.saturating_sub(len);
    let fill = " ".repeat(padding);
    let body = if spec.left {
        format!("{text}{fill}")
    } else {
        format!("{fill}{text}")
    };
    body.into_bytes()
}

fn pad_number(spec: &Spec, digits: &str) -> Vec<u8> {
    let len = digits.chars().count();
    if spec.zero && !spec.left && len < spec.width {
        // Zeros go after any sign.
        let (sign, rest) = match digits.strip_prefix(['-', '+']) {
            Some(rest) => (&digits[..1], rest),
            None => ("", digits),
        };
        let zeros = "0".repeat(spec.width - len);
        return format!("{sign}{zeros}{rest}").into_bytes();
    }
    pad_text(spec, digits)
}

/// Expands one backslash escape at the start of `chars`. Returns its bytes and
/// how many characters it used. An unknown escape is printed as it is.
fn escape(chars: &[char]) -> (Vec<u8>, usize) {
    let Some(&next) = chars.get(1) else {
        return (vec![b'\\'], 1);
    };
    let simple = match next {
        'a' => Some(0x07),
        'b' => Some(0x08),
        'f' => Some(0x0c),
        'n' => Some(b'\n'),
        'r' => Some(b'\r'),
        't' => Some(b'\t'),
        'v' => Some(0x0b),
        '\\' => Some(b'\\'),
        '"' => Some(b'"'),
        '\'' => Some(b'\''),
        _ => None,
    };
    if let Some(byte) = simple {
        return (vec![byte], 2);
    }
    if next.is_digit(8) {
        // `\NNN` takes up to three octal digits. `\0NNN` takes the zero and then
        // up to three more, so `\0101` is `A`, as in bash.
        let mut i = if next == '0' { 2 } else { 1 };
        let mut value = 0u32;
        let mut count = 0;
        while let Some(d) = chars
            .get(i)
            .and_then(|c| c.to_digit(8))
            .filter(|_| count < 3)
        {
            value = value * 8 + d;
            i += 1;
            count += 1;
        }
        return (vec![value as u8], i);
    }
    (vec![b'\\', next as u8], 2)
}

/// Expands the escapes of `echo -e`. `\c` ends the output, and it also drops the
/// trailing newline. The flag tells the caller to drop it.
pub(crate) fn echo_escapes(text: &str) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'c') {
            return (String::from_utf8_lossy(&out).into_owned(), true);
        }
        if chars[i] == '\\' {
            let (bytes, advance) = escape(&chars[i..]);
            out.extend(bytes);
            i += advance;
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), false)
}

/// Expands the escapes in a `%b` argument.
fn expand_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let (bytes, advance) = escape(&chars[i..]);
            out.extend(bytes);
            i += advance;
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(format_text: &str, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let formatted = format(format_text, &args).unwrap();
        assert!(formatted.error.is_none(), "{:?}", formatted.error);
        String::from_utf8(formatted.output).unwrap()
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(fmt("%s\\n", &["word"]), "word\n");
        assert_eq!(fmt("%s-%s\\n", &["a", "b", "c", "d"]), "a-b\nc-d\n");
        assert_eq!(fmt("x", &[]), "x");
        assert_eq!(fmt("%%\\n", &[]), "%\n");
    }

    #[test]
    fn widths_and_alignment() {
        assert_eq!(fmt("%5s|", &["ab"]), "   ab|");
        assert_eq!(fmt("%-5s|", &["ab"]), "ab   |");
    }

    #[test]
    fn numbers_and_bases() {
        assert_eq!(fmt("%d", &["42"]), "42");
        assert_eq!(fmt("%x", &["255"]), "ff");
        assert_eq!(fmt("%X", &["255"]), "FF");
        assert_eq!(fmt("%o", &["8"]), "10");
        assert_eq!(fmt("%05d", &["-42"]), "-0042");
        assert_eq!(fmt("%+d", &["7"]), "+7");
    }

    #[test]
    fn characters_and_missing_arguments() {
        assert_eq!(fmt("%c", &["Z"]), "Z");
        assert_eq!(fmt("[%s]", &[]), "[]");
        assert_eq!(fmt("[%d]", &[]), "[0]");
    }

    #[test]
    fn the_format_is_reused_for_remaining_arguments() {
        assert_eq!(fmt("%s\\n", &["one", "two"]), "one\ntwo\n");
    }

    #[test]
    fn a_bad_number_formats_as_zero_and_is_reported() {
        let args = vec!["abc".to_string()];
        let formatted = format("[%d]", &args).unwrap();
        assert_eq!(formatted.output, b"[0]");
        assert!(formatted.error.unwrap().contains("invalid number"));
    }

    #[test]
    fn an_unsupported_directive_is_an_error() {
        assert!(format("%f", &[]).unwrap_err().contains("unsupported"));
    }

    #[test]
    fn percent_b_expands_escapes_in_the_argument() {
        assert_eq!(fmt("%b", &["a\\tb"]), "a\tb");
    }
}
