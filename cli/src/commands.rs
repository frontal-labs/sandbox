use clap::{Parser, Subcommand};
use sandbox_session::{IsolateBackend, Session};
use sandbox_trace::{outcome_of, replay, Outcome, RunRecord, Trace};
use sandbox_types::{ErrorCode, ExecResult, SandboxError, SessionConfig};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The version the binary reports. Release builds set SANDBOX_VERSION from the git
/// tag, so `sandbox --version` matches the release. Other builds use the crate version.
const VERSION: &str = match option_env!("SANDBOX_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(name = "frontal-sandbox", version = VERSION, about = "Frontal Sandbox CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a command in a new sandbox. Arguments are joined with spaces and run
    /// as one shell script, so quote the whole script if it has spaces.
    Run {
        /// Write a trace of the script to this file. See `sandbox replay`.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Command to execute
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
    },

    /// Start an interactive shell in a new sandbox. Reads one script per line
    /// from stdin; `exit` or end of input stops it.
    Shell {
        /// Write a trace of every line to this file when the shell exits.
        #[arg(long)]
        trace: Option<PathBuf>,
    },

    /// Snapshot a session on the server named by SANDBOX_BASE_URL. Uses SANDBOX_API_KEY.
    Snapshot {
        /// Session ID
        session_id: String,
    },

    /// Fork a snapshot into sessions on the server named by SANDBOX_BASE_URL
    Fork {
        /// Snapshot ID
        snapshot_id: String,

        /// Number of forks
        #[arg(short, long, default_value = "1")]
        count: usize,
    },

    /// Replay a trace in a fresh sandbox and report every run whose output
    /// differs from the recorded one. Exits 1 when any run diverges.
    Replay {
        /// Trace file path
        trace_file: PathBuf,
    },

    /// Internal: serve one session as gRPC on a Unix socket, for a worker host.
    /// Unix only, because the socket is a Unix domain socket.
    #[cfg(unix)]
    #[command(hide = true)]
    Worker {
        /// Socket path to listen on. The host creates its directory with mode 0700.
        #[arg(long)]
        socket: PathBuf,
    },

    /// Start the sandbox server in the foreground. Prints one API key to stderr.
    Serve {
        /// Bind address
        #[arg(long, default_value = "0.0.0.0:8080")]
        addr: String,
        /// Keep keys, snapshots, and sleeping sessions in this directory, so they
        /// survive a restart (ADR 0008). Without it the server is in memory only.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Time a fixed set of operations on the local runtime. Prints one JSON line per case.
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
            Commands::Run { command, trace } => {
                run_script_once(&command.join(" "), trace.as_deref())
            }
            Commands::Shell { trace } => interactive_shell(trace.as_deref()),
            Commands::Replay { trace_file } => replay_trace(trace_file),
            Commands::Bench { name } => {
                let rt = runtime()?;
                rt.block_on(crate::bench::run(name.as_deref()))
            }
            #[cfg(unix)]
            Commands::Worker { socket } => {
                let rt = runtime()?;
                rt.block_on(sandbox_worker::serve(socket))?;
                Ok(0)
            }
            Commands::Serve { addr, data_dir } => serve(addr, data_dir.as_deref()),
            Commands::Snapshot { session_id } => snapshot(session_id),
            Commands::Fork { snapshot_id, count } => fork(snapshot_id, *count),
        }
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

fn run_script_once(script: &str, trace: Option<&Path>) -> Result<i32, SandboxError> {
    let rt = runtime()?;
    rt.block_on(async {
        let session = IsolateBackend::new(SessionConfig::default()).await?;
        let result = session.run_script(script).await;
        if let Some(path) = trace {
            let runs = vec![record(1, script, &result)];
            write_trace(path, runs)?;
        }
        let out = result?;
        print_output(&out.stdout, &out.stderr)
            .map_err(|e| SandboxError::internal(format!("failed to write output: {e}")))?;
        Ok(out.exit_code)
    })
}

fn interactive_shell(trace: Option<&Path>) -> Result<i32, SandboxError> {
    let rt = runtime()?;
    let session = rt.block_on(IsolateBackend::new(SessionConfig::default()))?;
    let mut runs = Vec::new();

    for line in std::io::stdin().lock().lines() {
        let line =
            line.map_err(|e| SandboxError::internal(format!("failed to read stdin: {e}")))?;
        if line.trim() == "exit" {
            break;
        }
        // A bad line reports its error and the session continues.
        let result = rt.block_on(session.run_script(&line));
        if trace.is_some() {
            runs.push(record(runs.len() as u64 + 1, &line, &result));
        }
        match result {
            Ok(out) => print_output(&out.stdout, &out.stderr)
                .map_err(|e| SandboxError::internal(format!("failed to write output: {e}")))?,
            Err(e) => eprintln!("Error: {}", e.message),
        }
    }
    if let Some(path) = trace {
        write_trace(path, runs)?;
    }
    Ok(0)
}

/// Replays a trace in one fresh sandbox, in the order it was recorded, and
/// prints every run whose outcome differs from the recording.
fn replay_trace(path: &Path) -> Result<i32, SandboxError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        SandboxError::new(
            ErrorCode::InvalidArgument,
            format!("cannot read {}: {e}", path.display()),
        )
    })?;
    let trace = Trace::parse(&text).map_err(|e| {
        SandboxError::new(
            ErrorCode::InvalidArgument,
            format!("{}: {e}", path.display()),
        )
    })?;

    let rt = runtime()?;
    let report = rt.block_on(async {
        let session = IsolateBackend::new(SessionConfig::default()).await?;
        let report = replay(&trace, |script| {
            let session = &session;
            async move {
                session
                    .run_script(&script)
                    .await
                    .map(|out| (out.exit_code, out.stdout, out.stderr))
            }
        })
        .await;
        Ok::<_, SandboxError>(report)
    })?;

    println!(
        "replayed {} runs: {} diverged",
        report.runs,
        report.divergences.len()
    );
    for d in &report.divergences {
        // Scripts come from the trace file, so they are escaped before printing.
        println!("run {} diverged: {}", d.seq, escaped(&d.script));
        println!("  expected: {}", describe(&d.expected));
        println!("  actual:   {}", describe(&d.actual));
    }
    Ok(if report.matches() { 0 } else { 1 })
}

/// One run as the trace records it.
fn record(seq: u64, script: &str, result: &Result<ExecResult, SandboxError>) -> RunRecord {
    let outcome = outcome_of(
        result
            .as_ref()
            .map(|out| (out.exit_code, out.stdout.clone(), out.stderr.clone()))
            .map_err(|e| SandboxError::new(e.code, e.message.clone())),
    );
    RunRecord {
        seq,
        script: script.to_string(),
        outcome,
    }
}

fn write_trace(path: &Path, runs: Vec<RunRecord>) -> Result<(), SandboxError> {
    std::fs::write(path, Trace { runs }.to_jsonl())
        .map_err(|e| SandboxError::internal(format!("cannot write trace {}: {e}", path.display())))
}

/// A short, readable form of an outcome for the replay report.
fn describe(outcome: &Outcome) -> String {
    const PREVIEW: usize = 120;
    match outcome {
        Outcome::Completed {
            exit_code,
            stdout,
            stderr,
        } => {
            let out = String::from_utf8_lossy(stdout);
            let err = String::from_utf8_lossy(stderr);
            format!(
                "exit {exit_code}, stdout {:?}, stderr {:?}",
                out.chars().take(PREVIEW).collect::<String>(),
                err.chars().take(PREVIEW).collect::<String>()
            )
        }
        Outcome::Failed { code, message } => format!("error {code}: {}", escaped(message)),
    }
}

/// Escapes control characters, so text from a trace or from a script cannot
/// move the cursor or set the title of the terminal it is printed to.
fn escaped(text: &str) -> String {
    text.escape_debug().collect()
}

/// Runs the server until the process is stopped. The key is printed once, to
/// stderr, because keys live in memory and nothing else can provision them.
fn serve(addr: &str, data_dir: Option<&Path>) -> Result<i32, SandboxError> {
    use std::sync::Arc;
    let scheduler = Arc::new(sandbox_scheduler::Scheduler::new());
    let rt = runtime()?;
    let (app, new_key) = match data_dir {
        Some(dir) => rt.block_on(sandbox_server::open_durable(scheduler, dir))?,
        None => {
            let auth = Arc::new(sandbox_server::auth::AuthManager::new());
            let key = auth.create_key();
            (sandbox_server::create_router(scheduler, auth), Some(key))
        }
    };
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| SandboxError::unavailable(format!("binding {addr}: {e}")))?;
        eprintln!("sandbox server listening on {addr}");
        if let Some(dir) = data_dir {
            eprintln!("state directory: {}", dir.display());
        }
        // Printed once, on the first run of a durable server, and on every run of
        // an in-memory one. Keys are stored as digests, so the key cannot be shown
        // again.
        if let Some(key) = &new_key {
            eprintln!("API key (shown once): {key}");
        }
        axum::serve(listener, app)
            .await
            .map_err(|e| SandboxError::unavailable(format!("server stopped: {e}")))
    })?;
    Ok(0)
}

fn client() -> Result<sandbox_sdk::Client, SandboxError> {
    sandbox_sdk::Client::from_env()
}

fn snapshot(session_id: &str) -> Result<i32, SandboxError> {
    let session: sandbox_types::SessionId = session_id
        .parse()
        .map_err(|_| SandboxError::invalid_argument(format!("not a session id: {session_id}")))?;
    let rt = runtime()?;
    let id = rt.block_on(client()?.snapshot(session))?;
    println!("{id}");
    Ok(0)
}

fn fork(snapshot_id: &str, count: usize) -> Result<i32, SandboxError> {
    let snapshot: sandbox_types::SnapshotId = snapshot_id
        .parse()
        .map_err(|_| SandboxError::invalid_argument(format!("not a snapshot id: {snapshot_id}")))?;
    let count =
        u32::try_from(count).map_err(|_| SandboxError::invalid_argument("count is too large"))?;
    let rt = runtime()?;
    let ids = rt.block_on(client()?.fork(snapshot, count))?;
    for id in ids {
        println!("{id}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_refuses_a_missing_trace_as_invalid_argument() {
        let replay = Cli::try_parse_from(["frontal-sandbox", "replay", "no-such-trace.jsonl"]).unwrap();
        assert_eq!(
            replay.execute().unwrap_err().code,
            sandbox_types::ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn run_returns_the_script_exit_code() {
        let ok = Cli::try_parse_from(["frontal-sandbox", "run", "true"]).unwrap();
        assert_eq!(ok.execute().unwrap(), 0);

        let failed = Cli::try_parse_from(["frontal-sandbox", "run", "false"]).unwrap();
        assert_eq!(failed.execute().unwrap(), 1);
    }

    #[test]
    fn run_reports_syntax_errors() {
        let cli = Cli::try_parse_from(["frontal-sandbox", "run", "echo", "'unterminated"]).unwrap();
        assert_eq!(
            cli.execute().unwrap_err().code,
            sandbox_types::ErrorCode::ShellError
        );
    }
}
