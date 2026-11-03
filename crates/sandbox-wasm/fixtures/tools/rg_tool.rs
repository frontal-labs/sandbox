// rg_tool - Simple search tool (like ripgrep)
// Searches for patterns in files

use std::fs;
use std::io::{self, Read};
use regex::Regex;
use walkdir::WalkDir;
use clap::Parser;

#[derive(Parser)]
#[command(name = "rg_tool", version, about = "Simple ripgrep-like tool")]
struct Args {
    /// Pattern to search for
    pattern: String,
    
    /// Files or directories to search (default: current directory)
    #[arg(default_value = ".")]
    paths: Vec<String>,
    
    /// Case insensitive search
    #[arg(short, long)]
    ignore_case: bool,
    
    /// Show line numbers
    #[arg(short, long)]
    line_number: bool,
    
    /// Only show matching part
    #[arg(short = 'o', long)]
    only_matching: bool,
    
    /// Count matches per file
    #[arg(short, long)]
    count: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    let regex = if args.ignore_case {
        Regex::new(&format!("(?i){}", args.pattern))?
    } else {
        Regex::new(&args.pattern)?
    };
    
    let mut total_matches = 0;
    
    for path in &args.paths {
        let metadata = fs::metadata(path)?;
        if metadata.is_file() {
            total_matches += search_file(path, &regex, &args)?;
        } else if metadata.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    total_matches += search_file(entry.path().to_str().unwrap(), &regex, &args)?;
                }
            }
        }
    }
    
    if args.count {
        println!("Total matches: {}", total_matches);
    }
    
    Ok(())
}

fn search_file(path: &str, regex: &Regex, args: &Args) -> anyhow::Result<usize> {
    let content = fs::read_to_string(path)?;
    let mut matches = 0;

    let matching_lines = content
        .lines()
        .enumerate()
        .filter(|(_, line)| regex.is_match(line));
    for (line_num, line) in matching_lines {
        matches += 1;
        // With -c only the count is printed, so matching lines are not printed.
        if !args.count {
            let prefix = if args.paths.len() > 1 {
                format!("{}:", path)
            } else {
                String::new()
            };

            let line_prefix = if args.line_number {
                format!("{}:", line_num + 1)
            } else {
                String::new()
            };

            if args.only_matching {
                for mat in regex.find_iter(line) {
                    println!("{}{}{}", prefix, line_prefix, mat.as_str());
                }
            } else {
                println!("{}{}{}", prefix, line_prefix, line);
            }
        }
    }

    Ok(matches)
}