// git_tool - Simple Git tool stub (demonstrates WASM tool pattern)
// This is a minimal implementation showing the command structure.
// A full implementation would use gix (pure Rust Git) but requires Rust 1.96+.
use std::env;
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let cmd = &args[1];
    let result = match cmd.as_str() {
        "init" => cmd_init(&args[2..]),
        "status" => cmd_status(&args[2..]),
        "add" => cmd_add(&args[2..]),
        "commit" => cmd_commit(&args[2..]),
        "log" => cmd_log(&args[2..]),
        "clone" => cmd_clone(&args[2..]),
        _ => {
            eprintln!("git_tool: unknown command '{}'", cmd);
            print_usage();
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("git_tool: {}", e);
        std::process::exit(1);
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Usage: git_tool <command> [args...]");
    eprintln!("Commands:");
    eprintln!("  init                    Initialize a new repository");
    eprintln!("  status                  Show working tree status");
    eprintln!("  add <path>...           Add files to index");
    eprintln!("  commit -m <message>     Commit staged changes");
    eprintln!("  log [--oneline]         Show commit history");
    eprintln!("  clone <url> <path>      Clone a repository (not implemented in WASM)");
}

fn cmd_init(_args: &[String]) -> Result<(), String> {
    // In a real implementation, this would create .git directory
    println!("Initialized empty Git repository in .git/");
    Ok(())
}

fn cmd_status(_args: &[String]) -> Result<(), String> {
    println!("On branch main");
    println!("No commits yet");
    println!("nothing to commit (create/copy files and use \"git add\" to track)");
    Ok(())
}

fn cmd_add(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("add: missing path".to_string());
    }
    println!("Added {} file(s) (stub)", args.len());
    Ok(())
}

fn cmd_commit(args: &[String]) -> Result<(), String> {
    let mut message = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-m" {
            if i + 1 < args.len() {
                message = Some(args[i + 1].clone());
                i += 2;
            } else {
                return Err("commit: option -m requires an argument".to_string());
            }
        } else {
            i += 1;
        }
    }
    
    let message = message.ok_or_else(|| "commit: missing -m <message>".to_string())?;
    println!("Created commit abc1234 (stub): {}", message);
    Ok(())
}

fn cmd_log(args: &[String]) -> Result<(), String> {
    let oneline = args.contains(&"--oneline".to_string());
    if oneline {
        println!("abc1234 Initial commit (stub)");
    } else {
        println!("commit abc1234");
        println!("Author: Sandbox User <sandbox@example.com>");
        println!("Date:   Mon Jan 1 00:00:00 2024 +0000");
        println!();
        println!("    Initial commit (stub)");
        println!();
    }
    Ok(())
}

fn cmd_clone(_args: &[String]) -> Result<(), String> {
    Err("clone: not implemented (network access not available in WASM)".to_string())
}