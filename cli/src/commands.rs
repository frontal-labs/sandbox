use clap::{Parser, Subcommand};
use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{SandboxError, SessionConfig};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sandbox", version, about = "Frontal Sandbox CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a command in a new sandbox. Arguments are joined with spaces and run
    /// as one shell script, so quote the whole script if it has spaces.
    Run {
        /// Command to execute
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
    },

    /// Start an interactive shell in a new sandbox. Reads one script per line
    /// from stdin; `exit` or end of input stops it.
    Shell,

    /// Create a snapshot of a session (not implemented yet)
    Snapshot {
        /// Session ID
        session_id: String,
    },

    /// Fork a snapshot into multiple sessions (not implemented yet)
    Fork {
        /// Snapshot ID
        snapshot_id: String,

        /// Number of forks
        #[arg(short, long, default_value = "1")]
        count: usize,
    },

    /// Replay a trace (not implemented yet)
    Replay {
        /// Trace file path
        trace_file: PathBuf,
    },

    /// Internal: serve one session over stdin and stdout for a worker host.
    #[command(hide = true)]
    Worker,

    /// Start the sandbox server (not implemented yet)
    Serve {
        /// Bind address
        #[arg(long, default_value = "0.0.0.0:8080")]
        addr: String,
    },

    /// Run benchmarks (not implemented yet)
    Bench {
        /// Benchmark name
        #[arg(short, long)]
        name: Option<String>,
    },
}

impl Cli {
    /// Parses arguments, runs the command, and exits with its status.
    pub fn run() {
        let cli = Cli::parse();
        match cli.execute() {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("Error: {}", e.message);
                std::process::exit(1);
            }
        }
    }

    /// Runs the command and returns the process exit code.
    pub fn execute(&self) -> Result<i32, SandboxError> {
        match &self.command {
            Commands::Run { command } => run_script_once(&command.join(" ")),
            Commands::Shell => interactive_shell(),
            Commands::Worker => {
                sandbox_worker::run_stdio_worker()?;
                Ok(0)
            }
            other => Err(SandboxError::unsupported(format!(
                "`sandbox {}` is not implemented yet",
                subcommand_name(other)
            ))),
        }
    }
}

fn subcommand_name(command: &Commands) -> &'static str {
    match command {
        Commands::Run { .. } => "run",
        Commands::Shell => "shell",
        Commands::Worker => "worker",
        Commands::Snapshot { .. } => "snapshot",
        Commands::Fork { .. } => "fork",
        Commands::Replay { .. } => "replay",
        Commands::Serve { .. } => "serve",
        Commands::Bench { .. } => "bench",
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, SandboxError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SandboxError::internal(format!("failed to start runtime: {e}")))
}

fn print_output(stdout: &[u8], stderr: &[u8]) -> std::io::Result<()> {
    std::io::stdout().write_all(stdout)?;
    std::io::stdout().flush()?;
    std::io::stderr().write_all(stderr)
}

fn run_script_once(script: &str) -> Result<i32, SandboxError> {
    let rt = runtime()?;
    rt.block_on(async {
        let session = IsolateBackend::new(SessionConfig::default()).await?;
        let out = session.run_script(script).await?;
        print_output(&out.stdout, &out.stderr)
            .map_err(|e| SandboxError::internal(format!("failed to write output: {e}")))?;
        Ok(out.exit_code)
    })
}

fn interactive_shell() -> Result<i32, SandboxError> {
    let rt = runtime()?;
    let session = rt.block_on(IsolateBackend::new(SessionConfig::default()))?;

    for line in std::io::stdin().lock().lines() {
        let line =
            line.map_err(|e| SandboxError::internal(format!("failed to read stdin: {e}")))?;
        if line.trim() == "exit" {
            break;
        }
        // A bad line reports its error and the session continues.
        match rt.block_on(session.run_script(&line)) {
            Ok(out) => print_output(&out.stdout, &out.stderr)
                .map_err(|e| SandboxError::internal(format!("failed to write output: {e}")))?,
            Err(e) => eprintln!("Error: {}", e.message),
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_subcommands_return_errors_not_success() {
        let cli = Cli::try_parse_from(["sandbox", "serve"]).unwrap();
        let err = cli.execute().unwrap_err();
        assert_eq!(err.code, sandbox_types::ErrorCode::Unsupported);
    }

    #[test]
    fn run_returns_the_script_exit_code() {
        let ok = Cli::try_parse_from(["sandbox", "run", "true"]).unwrap();
        assert_eq!(ok.execute().unwrap(), 0);

        let failed = Cli::try_parse_from(["sandbox", "run", "false"]).unwrap();
        assert_eq!(failed.execute().unwrap(), 1);
    }

    #[test]
    fn run_reports_syntax_errors() {
        let cli = Cli::try_parse_from(["sandbox", "run", "echo", "'unterminated"]).unwrap();
        assert_eq!(
            cli.execute().unwrap_err().code,
            sandbox_types::ErrorCode::ShellError
        );
    }
}
