//! Test fixture for the WASI host. Each subcommand exercises one filesystem
//! or stdio call that real programs use.

use std::fs;
use std::io::{self, Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("args") => {
            println!("{}", args[1..].join(","));
            0
        }
        Some("cat") => report(fs::read(&args[2]).map(|data| {
            io::stdout().write_all(&data).unwrap();
        })),
        Some("write") => report(fs::write(&args[2], args[3].as_bytes())),
        Some("append") => report(
            fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&args[2])
                .and_then(|mut f| f.write_all(args[3].as_bytes())),
        ),
        Some("ls") => report(fs::read_dir(&args[2]).map(|entries| {
            let mut names: Vec<String> = entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            for name in names {
                println!("{name}");
            }
        })),
        Some("stat") => report(fs::metadata(&args[2]).map(|m| {
            println!("{} {}", if m.is_dir() { "dir" } else { "file" }, m.len());
        })),
        Some("mkdir") => report(fs::create_dir(&args[2])),
        Some("rm") => report(fs::remove_file(&args[2])),
        Some("rmdir") => report(fs::remove_dir(&args[2])),
        Some("mv") => report(fs::rename(&args[2], &args[3])),
        Some("stdin") => {
            let mut data = Vec::new();
            io::stdin().read_to_end(&mut data).unwrap();
            io::stdout().write_all(&data).unwrap();
            0
        }
        Some("stderr") => {
            eprintln!("to stderr");
            0
        }
        Some("exit") => args[2].parse().unwrap_or(1),
        _ => {
            eprintln!("usage: wasi_tool COMMAND ...");
            2
        }
    };
    std::process::exit(code);
}

/// Prints the error (if any) to stderr and returns the exit code.
fn report(result: io::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
