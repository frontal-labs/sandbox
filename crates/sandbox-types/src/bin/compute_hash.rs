//! Compute BLAKE3 hashes of WASM tools
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths: Vec<String> = env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("Usage: compute_hash <file>...");
        return ExitCode::FAILURE;
    }

    let mut failed = false;
    for path in &paths {
        match fs::read(path) {
            Ok(data) => println!("{}  {}", blake3::hash(&data).to_hex(), path),
            Err(err) => {
                eprintln!("{path}: {err}");
                failed = true;
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
