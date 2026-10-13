use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sandbox", version, about = "Frontal Sandbox CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a command in a new sandbox
    Run {
        /// Command to execute
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
        
        /// Working directory
        #[arg(short, long, default_value = "/work")]
        cwd: String,
        
        /// Memory limit in MB
        #[arg(long, default_value = "512")]
        memory: u64,
        
        /// CPU limit in milliseconds per second
        #[arg(long, default_value = "1000")]
        cpu: u64,
    },
    
    /// Start an interactive shell
    Shell {
        /// Working directory
        #[arg(short, long, default_value = "/work")]
        cwd: String,
    },
    
    /// Create a snapshot of a session
    Snapshot {
        /// Session ID
        session_id: String,
    },
    
    /// Fork a snapshot into multiple sessions
    Fork {
        /// Snapshot ID
        snapshot_id: String,
        
        /// Number of forks
        #[arg(short, long, default_value = "1")]
        count: usize,
    },
    
    /// Replay a trace
    Replay {
        /// Trace file path
        trace_file: PathBuf,
    },
    
    /// Start the sandbox server
    Serve {
        /// Bind address
        #[arg(long, default_value = "0.0.0.0:8080")]
        addr: String,
    },
    
    /// Run benchmarks
    Bench {
        /// Benchmark name
        #[arg(short, long)]
        name: Option<String>,
    },
}

impl Cli {
    pub fn run() {
        let cli = Cli::parse();
        if let Err(e) = cli.execute() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    pub fn execute(&self) -> Result<(), sandbox_types::SandboxError> {
        match &self.command {
            Commands::Run { command, cwd, memory, cpu } => {
                println!("Running command: {} in {}", command.join(" "), cwd);
                println!("Memory: {} MB, CPU: {} ms/s", memory, cpu);
                // TODO: Implement actual execution
            }
            Commands::Shell { cwd } => {
                println!("Starting shell in {}", cwd);
                // TODO: Implement shell
            }
            Commands::Snapshot { session_id } => {
                println!("Creating snapshot for session: {}", session_id);
            }
            Commands::Fork { snapshot_id, count } => {
                println!("Forking snapshot {} into {} sessions", snapshot_id, count);
            }
            Commands::Replay { trace_file } => {
                println!("Replaying trace from {:?}", trace_file);
            }
            Commands::Serve { addr } => {
                println!("Starting server on {}", addr);
            }
            Commands::Bench { name } => {
                println!("Running benchmarks: {:?}", name);
            }
        }
        Ok(())
    }
}