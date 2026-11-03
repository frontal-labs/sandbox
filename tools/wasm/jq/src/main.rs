//! `jq`: a JSON processor for the sandbox, built on jaq (a pure-Rust jq clone).
//!
//! Usage: `jq [-r] [-c] [-n] FILTER [FILE...]`. Input comes from the files, or
//! from stdin when none are given. `-r` prints strings raw, `-c` prints compact
//! JSON, and `-n` runs the filter once on `null` instead of on the input.
//!
//! Exit codes follow jq: 0 on success, 2 for a usage or file error, 3 for a
//! filter that does not compile, 5 when a run raised an error.

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::{read, write, Val};
use std::io::{self, Read, Write};
use std::process::ExitCode;

const USAGE_ERROR: u8 = 2;
const COMPILE_ERROR: u8 = 3;
const RUNTIME_ERROR: u8 = 5;

#[derive(Default)]
struct Options {
    raw: bool,
    compact: bool,
    null_input: bool,
    filter: Option<String>,
    files: Vec<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut only_positional = false;
    for arg in args {
        match arg.as_str() {
            "--" if !only_positional => only_positional = true,
            "-r" | "--raw-output" if !only_positional => opts.raw = true,
            "-c" | "--compact-output" if !only_positional => opts.compact = true,
            "-n" | "--null-input" if !only_positional => opts.null_input = true,
            flag if !only_positional && flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option: {flag}"));
            }
            _ if opts.filter.is_none() => opts.filter = Some(arg),
            _ => opts.files.push(arg),
        }
    }
    Ok(opts)
}

fn compile(code: &str) -> Result<jaq_core::Filter<data::JustLut<Val>>, String> {
    // The core layer, the standard library, and the JSON layer, as the reference jaq
    // CLI wires them. Each layer's definitions use the natives of the others.
    let loader = Loader::new(jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs()));
    let arena = Arena::default();
    let modules = loader
        .load(&arena, File { code, path: () })
        .map_err(|e| format!("syntax error: {e:?}"))?;
    Compiler::default()
        .with_funs(jaq_core::funs::<data::JustLut<Val>>().chain(jaq_std::funs()).chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| format!("compile error: {e:?}"))
}

/// Reads every input file, or stdin when there are none, as one byte buffer.
fn read_input(files: &[String]) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if files.is_empty() {
        io::stdin().lock().read_to_end(&mut buf)?;
    }
    for file in files {
        std::fs::File::open(file)
            .map_err(|e| io::Error::new(e.kind(), format!("{file}: {e}")))?
            .read_to_end(&mut buf)?;
        buf.push(b'\n');
    }
    Ok(buf)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err((code, message)) => {
            eprintln!("jq: error: {message}");
            ExitCode::from(code)
        }
    }
}

fn run() -> Result<ExitCode, (u8, String)> {
    let opts = parse_args(std::env::args().skip(1)).map_err(|e| (USAGE_ERROR, e))?;
    let Some(code) = opts.filter.as_deref() else {
        return Err((USAGE_ERROR, "usage: jq [-r] [-c] [-n] FILTER [FILE...]".to_string()));
    };
    let filter = compile(code).map_err(|e| (COMPILE_ERROR, e))?;
    let input = read_input(&opts.files).map_err(|e| (USAGE_ERROR, e.to_string()))?;

    let pp = write::Pp {
        indent: (!opts.compact).then(|| "  ".to_string()),
        // jq puts a space after the colon in pretty output, and none in compact output.
        sep_space: !opts.compact,
        ..write::Pp::default()
    };
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut failed = false;

    let inputs: Vec<Result<Val, String>> = if opts.null_input {
        vec![Ok(Val::default())]
    } else {
        read::parse_many(&input)
            .map(|v| v.map_err(|e| e.to_string()))
            .collect()
    };

    for item in inputs {
        let value = item.map_err(|e| (RUNTIME_ERROR, format!("invalid JSON input: {e}")))?;
        let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
        for result in filter.id.run((ctx, value)).map(unwrap_valr) {
            match result {
                Ok(v) => emit(&mut out, &pp, opts.raw, &v).map_err(|e| (RUNTIME_ERROR, e.to_string()))?,
                Err(e) => {
                    failed = true;
                    eprintln!("jq: error: {e}");
                }
            }
        }
    }
    out.flush().map_err(|e| (RUNTIME_ERROR, e.to_string()))?;
    Ok(if failed { ExitCode::from(RUNTIME_ERROR) } else { ExitCode::SUCCESS })
}

fn emit(out: &mut impl Write, pp: &write::Pp, raw: bool, v: &Val) -> io::Result<()> {
    match (raw, v) {
        // A text string is written as its bytes, without quotes or escapes.
        (true, Val::TStr(s)) => out.write_all(s)?,
        _ => write::write(out, pp, 0, v)?,
    }
    writeln!(out)
}
