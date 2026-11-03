// fd_tool - Simple find tool (like fd)
// Finds files by name pattern

use std::fs;
use walkdir::WalkDir;
use clap::Parser;

#[derive(Parser)]
#[command(name = "fd_tool", version, about = "Simple fd-like find tool")]
struct Args {
    /// Pattern to match (substring match)
    pattern: String,
    
    /// Directory to search (default: current directory)
    #[arg(default_value = ".")]
    path: String,
    
    /// Only show files (not directories)
    #[arg(short, long)]
    type_file: bool,
    
    /// Only show directories
    #[arg(short, long)]
    type_dir: bool,
    
    /// Case insensitive
    #[arg(short, long)]
    ignore_case: bool,
    
    /// Max depth
    #[arg(short, long)]
    max_depth: Option<usize>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    let pattern = if args.ignore_case {
        args.pattern.to_lowercase()
    } else {
        args.pattern
    };
    
    let mut walker = WalkDir::new(&args.path);
    if let Some(depth) = args.max_depth {
        walker = walker.max_depth(depth);
    }
    
    let wanted = walker.into_iter().filter_map(|e| e.ok()).filter(|entry| {
        let file_type = entry.file_type();
        (!args.type_file || file_type.is_file()) && (!args.type_dir || file_type.is_dir())
    });
    for entry in wanted {
        let file_type = entry.file_type();
        
        
        let name = entry.file_name().to_string_lossy();
        let haystack = if args.ignore_case {
            name.to_lowercase()
        } else {
            name.to_string()
        };
        
        if haystack.contains(&pattern) {
            println!("{}", entry.path().display());
        }
    }
    
    Ok(())
}