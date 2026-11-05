use clap::{Parser, Subcommand};
use sandbox_session::{changed_since, committed_root, replay_run, IsolateBackend, Session};
use sandbox_trace::{outcome_of, replay, FileChange, FsOp, Outcome, RunRecord, Trace};
use sandbox_types::{ErrorCode, ExecResult, SandboxError, SessionConfig};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The version the binary reports. Release builds set SANDBOX_VERSION from the git
/// tag, so `sandbox --version` matches the release. Other builds use the crate version.
const VERSION: &str = match option_env!("SANDBOX_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Subcommand)]
pub enum SandboxCommand {
    /// Print the session's environment (/etc/agent/env.json) as JSON.
    Info {
        /// Session ID
        session_id: String,
    },

    /// Pin the session's current state as a checkpoint, and print its root.
    Checkpoint {
        /// Session ID
        session_id: String,
    },

    /// Restore the filesystem from N checkpoints ago. The current state is kept as a
    /// checkpoint first. Processes are not kept.
    Undo {
        /// Session ID
        session_id: String,

        /// How many checkpoints back to restore
        #[arg(default_value = "1")]
        steps: u32,
    },

    /// Fork the session's current state into N children and run a command in each.
    /// Prints one line per child. Exits 1 when a child's command failed.
    ///
    /// Example: frontal-sandbox sandbox fork <session> -c 3 -- touch /r
    Fork {
        /// Session ID
        session_id: String,

        /// Number of children
        #[arg(short, long, default_value = "1")]
        count: u32,

        /// The command to run in each child
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

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

    /// Tier 1 session tools: environment, checkpoints, undo, and fork-with-command.
    /// Uses SANDBOX_BASE_URL and SANDBOX_API_KEY.
    Sandbox {
        #[command(subcommand)]
        command: SandboxCommand,
    },

    /// Replay a trace in a fresh sandbox and report every run whose output
    /// differs from the recorded one. Exits 1 when any run diverges.
    Replay {
        /// Trace file path
        trace_file: PathBuf,
    },

    /// Internal: run one `node` script from stdin in a contained child process.
    /// Frames go to stdout. See `sandbox_worker::script`.
    #[cfg(unix)]
    #[command(hide = true)]
    ScriptRun {
        /// Unshare the network, IPC, UTS, and mount namespaces before running.
        #[arg(long)]
        namespaces: bool,
        /// Install the seccomp denylist before running.
        #[arg(long)]
        seccomp: bool,
    },

    /// Internal: serve one session as gRPC on a Unix socket, for a worker host.
    /// Unix only, because the socket is a Unix domain socket.
    #[cfg(unix)]
    #[command(hide = true)]
    Worker {
        /// Socket path to listen on. The host creates its directory with mode 0700.
        #[arg(long)]
        socket: PathBuf,
        /// Unshare the network, IPC, UTS, and mount namespaces before serving.
        #[arg(long)]
        namespaces: bool,
        /// Install the seccomp denylist before serving.
        #[arg(long)]
        seccomp: bool,
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
        /// Collect unreachable blobs every this many seconds (ADR 0013). 0 turns
        /// the schedule off, and `POST /v1/gc` remains available.
        #[arg(long, default_value_t = 600)]
        gc_interval_secs: u64,
        /// Put a session to sleep after this many seconds with no request (PLAN
        /// §4.14). A request to a sleeping session wakes it. 0 turns idle sleep off.
        #[arg(long, default_value_t = 300)]
        idle_sleep_secs: u64,
        /// Keep the state in this Postgres database, shared by every node that uses
        /// it, instead of in `--data-dir` alone (PLAN §4.8). `--data-dir` still holds
        /// the blobs, so it must be a directory every node can reach.
        #[arg(long, requires = "data_dir")]
        database_url: Option<String>,
        /// A directory with `vmlinuz` and `initrd.gz`, the guest image (see
        /// docker/emulated-vm/build-image.sh). With it, a command the isolate tier cannot
        /// run moves the session to a Tier 2 VM. Without it, such a command is refused.
        #[arg(long)]
        vm_image: Option<PathBuf>,
        /// The QEMU binary that runs the guest.
        #[arg(long, default_value = "qemu-system-x86_64")]
        qemu: PathBuf,
        /// How QEMU runs the guest: `tcg` (software, works in Docker without KVM) or `kvm`.
        #[arg(long, default_value = "tcg")]
        vm_accel: String,
        /// Guest memory in MiB.
        #[arg(long, default_value_t = 2048)]
        vm_memory_mib: u32,
        /// Seconds to wait for a guest's agent to answer after boot.
        #[arg(long, default_value_t = 180)]
        vm_boot_timeout_secs: u64,
        /// Guests kept booted ahead of promotions, so a promotion skips the boot.
        #[arg(long, default_value_t = 1)]
        vm_warm_guests: usize,
        /// A directory with `python.wasm` and `python313.zip` (tools/wasm/python/build.sh). New
        /// isolate sessions get CPython installed, so `python` runs in them. Not yet combined
        /// with --data-dir or a VM image.
        #[arg(long)]
        python_artifacts: Option<PathBuf>,
        /// An OTLP/HTTP collector to send one span per request to, such as
        /// http://127.0.0.1:4318. The OTEL_EXPORTER_OTLP_ENDPOINT variable does the same.
        #[arg(long)]
        otel_endpoint: Option<String>,
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
        if let Err(e) = install_script_runner(&cli.command) {
            eprintln!("Error: {}", e.message);
            std::process::exit(1);
        }
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
            Commands::ScriptRun {
                namespaces,
                seccomp,
            } => {
                // Applies the isolation before any thread exists. See `run_script_child`.
                let isolation = sandbox_worker::Isolation {
                    namespaces: *namespaces,
                    seccomp: *seccomp,
                    cgroup: None,
                };
                sandbox_worker::run_script_child(&isolation)?;
                Ok(0)
            }
            #[cfg(unix)]
            Commands::Worker {
                socket,
                namespaces,
                seccomp,
            } => {
                // Before the runtime exists: namespaces and seccomp apply to the
                // calling thread and to threads created later.
                let isolation = sandbox_worker::Isolation {
                    namespaces: *namespaces,
                    seccomp: *seccomp,
                    cgroup: None,
                };
                sandbox_worker::enter(&isolation)?;
                let rt = runtime()?;
                rt.block_on(sandbox_worker::serve(socket))?;
                Ok(0)
            }
            Commands::Serve {
                addr,
                data_dir,
                gc_interval_secs,
                idle_sleep_secs,
                database_url,
                vm_image,
                qemu,
                vm_accel,
                vm_memory_mib,
                vm_boot_timeout_secs,
                vm_warm_guests,
                python_artifacts,
                otel_endpoint,
            } => {
                // The URL may also come from the environment, so a secret need not be on the command line.
                let url = database_url
                    .clone()
                    .or_else(|| std::env::var("SANDBOX_DATABASE_URL").ok());
                let vm = match vm_image {
                    None => None,
                    Some(image) => Some(sandbox_server::VmConfig {
                        qemu: qemu.clone(),
                        image: image.clone(),
                        accel: match vm_accel.as_str() {
                            "tcg" => sandbox_vmm::Accel::Tcg,
                            "kvm" => sandbox_vmm::Accel::Kvm,
                            other => {
                                return Err(SandboxError::invalid_argument(format!(
                                    "--vm-accel must be tcg or kvm, not {other}"
                                )))
                            }
                        },
                        memory_mib: *vm_memory_mib,
                        boot_timeout: std::time::Duration::from_secs(*vm_boot_timeout_secs),
                        warm_guests: *vm_warm_guests,
                    }),
                };
                let python = match python_artifacts {
                    None => None,
                    Some(dir) => Some(sandbox_wasm::PythonArtifacts::load(dir)?),
                };
                let otel = otel_endpoint
                    .clone()
                    .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok());
                let telemetry = otel
                    .as_deref()
                    .map(sandbox_server::otel::SpanExporter::start);
                serve(
                    addr,
                    data_dir.as_deref(),
                    *gc_interval_secs,
                    *idle_sleep_secs,
                    url.as_deref(),
                    vm,
                    python,
                    telemetry,
                )
            }
            Commands::Snapshot { session_id } => snapshot(session_id),
            Commands::Fork { snapshot_id, count } => fork(snapshot_id, *count),
            Commands::Sandbox { command } => sandbox_session(command),
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
        let (random_state, result, files) = run_traced(&session, script).await;
        if let Some(path) = trace {
            let runs = vec![record(1, script, &result, files, random_state)];
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

    let lines = std::io::stdin()
        .lock()
        .lines()
        .map(|line| line.map_err(|e| SandboxError::internal(format!("failed to read stdin: {e}"))))
        .take_while(|line| !matches!(line, Ok(text) if text.trim() == "exit"));
    for line in lines {
        let line = line?;
        // A bad line reports its error and the session continues.
        let (random_state, result, files) = rt.block_on(run_traced(&session, &line));
        if trace.is_some() {
            runs.push(record(
                runs.len() as u64 + 1,
                &line,
                &result,
                files,
                random_state,
            ));
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
        let report = replay(&trace, |run| {
            let session = &session;
            let run = run.clone();
            async move { replay_run(session, &run).await }
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
        let what = match &d.fs {
            Some(FsOp::Write { path, data }) => {
                format!("write {} ({} bytes)", escaped(path), data.len())
            }
            Some(FsOp::Delete { path }) => format!("delete {}", escaped(path)),
            None => escaped(&d.script),
        };
        println!("run {} diverged: {what}", d.seq);
        println!("  expected: {}", describe(&d.expected));
        println!("  actual:   {}", describe(&d.actual));
    }
    Ok(if report.matches() { 0 } else { 1 })
}

/// Runs one script and returns its result with the files it changed.
async fn run_traced(
    session: &IsolateBackend,
    script: &str,
) -> (
    Option<u64>,
    Result<ExecResult, SandboxError>,
    Result<Vec<FileChange>, SandboxError>,
) {
    let before = committed_root(session).await;
    // The generator state before the run, so a replay draws the same $RANDOM values.
    let random_state = Some(session.random_state().await);
    let result = session.run_script(script).await;
    let files = match before {
        Ok(before) => changed_since(session, before)
            .await
            .map(|changes| changes.into_iter().map(FileChange::from).collect()),
        Err(e) => Err(e),
    };
    (random_state, result, files)
}

/// One run as the trace records it. Files that could not be read leave the run
/// without its file list, and the error is reported on stderr.
fn record(
    seq: u64,
    script: &str,
    result: &Result<ExecResult, SandboxError>,
    files: Result<Vec<FileChange>, SandboxError>,
    random_state: Option<u64>,
) -> RunRecord {
    let files = files.unwrap_or_else(|e| {
        eprintln!(
            "Warning: the trace does not list the files this run changed: {}",
            e.message
        );
        Vec::new()
    });
    let outcome = outcome_of(
        result
            .as_ref()
            .map(|out| (out.exit_code, out.stdout.clone(), out.stderr.clone()))
            .map_err(|e| SandboxError::new(e.code, e.message.clone())),
    );
    RunRecord {
        seq,
        script: script.to_string(),
        fs: None,
        outcome,
        files,
        random_state,
    }
}

fn write_trace(path: &Path, runs: Vec<RunRecord>) -> Result<(), SandboxError> {
    let text = Trace { runs }
        .to_jsonl()
        .map_err(|e| SandboxError::internal(format!("cannot encode the trace: {e}")))?;
    std::fs::write(path, text)
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

/// Opens the Postgres-backed state. A build without the `clustered` feature refuses.
#[cfg(feature = "clustered")]
fn clustered_open(
    rt: &tokio::runtime::Runtime,
    scheduler: std::sync::Arc<sandbox_scheduler::Scheduler>,
    url: &str,
    dir: &Path,
    vm: Option<sandbox_server::VmConfig>,
) -> Result<(axum::Router, sandbox_server::AppState, Option<String>), SandboxError> {
    rt.block_on(sandbox_server::open_clustered_with_vm(
        scheduler, url, dir, vm,
    ))
}

#[cfg(not(feature = "clustered"))]
fn clustered_open(
    _rt: &tokio::runtime::Runtime,
    _scheduler: std::sync::Arc<sandbox_scheduler::Scheduler>,
    _url: &str,
    _dir: &Path,
    _vm: Option<sandbox_server::VmConfig>,
) -> Result<(axum::Router, sandbox_server::AppState, Option<String>), SandboxError> {
    Err(SandboxError::new(
        ErrorCode::Unsupported,
        "--database-url needs a build with the clustered feature: cargo build --features clustered",
    ))
}

/// Runs the server until the process is stopped. The key is printed once, to
/// stderr, because keys live in memory and nothing else can provision them.
fn serve(
    addr: &str,
    data_dir: Option<&Path>,
    gc_interval_secs: u64,
    idle_sleep_secs: u64,
    database_url: Option<&str>,
    vm: Option<sandbox_server::VmConfig>,
    python: Option<sandbox_wasm::PythonArtifacts>,
    telemetry: Option<sandbox_server::otel::SpanExporter>,
) -> Result<i32, SandboxError> {
    use std::sync::Arc;
    // Log to stderr, so stdout keeps its key line. RUST_LOG overrides the default.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
    let scheduler = Arc::new(sandbox_scheduler::Scheduler::new());
    let rt = runtime()?;
    let (app, state, new_key) = match (data_dir, database_url) {
        (Some(_), Some(_)) if telemetry.is_some() => {
            return Err(sandbox_types::SandboxError::new(
                sandbox_types::ErrorCode::Unsupported,
                "span export is not yet wired into the clustered server",
            ))
        }
        (Some(dir), Some(url)) => clustered_open(&rt, scheduler, url, dir, vm)?,
        (Some(dir), None) => rt.block_on(sandbox_server::open_durable_with_options(
            scheduler,
            dir,
            sandbox_server::ServerOptions {
                vm,
                python,
                telemetry,
            },
        ))?,
        (None, _) => {
            let auth = Arc::new(sandbox_server::auth::AuthManager::new());
            let key = auth.create_key();
            let (app, state) = sandbox_server::create_router_with_options(
                scheduler,
                auth,
                sandbox_server::ServerOptions {
                    vm,
                    python,
                    telemetry,
                },
            );
            (app, state, Some(key))
        }
    };
    rt.block_on(async {
        if gc_interval_secs > 0 {
            sandbox_server::gc::schedule(
                state.clone(),
                std::time::Duration::from_secs(gc_interval_secs),
            );
        }
        if idle_sleep_secs > 0 {
            // Sweep at least once a second and at most every 30 seconds, so a
            // session sleeps soon after its idle time, not much later.
            let every = std::time::Duration::from_secs(idle_sleep_secs.clamp(1, 30));
            sandbox_server::idle::schedule(
                state,
                std::time::Duration::from_secs(idle_sleep_secs),
                every,
            );
        }
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

fn client() -> Result<frontal_sandbox_sdk::Client, SandboxError> {
    frontal_sandbox_sdk::Client::from_env()
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

fn parse_session(raw: &str) -> Result<sandbox_types::SessionId, SandboxError> {
    raw.parse()
        .map_err(|_| SandboxError::invalid_argument(format!("not a session id: {raw}")))
}

fn sandbox_session(command: &SandboxCommand) -> Result<i32, SandboxError> {
    let rt = runtime()?;
    match command {
        SandboxCommand::Info { session_id } => {
            let env = rt.block_on(client()?.info(parse_session(session_id)?))?;
            println!("{}", serde_json::to_string_pretty(&env).unwrap_or_default());
            Ok(0)
        }
        SandboxCommand::Checkpoint { session_id } => {
            let pinned = rt.block_on(client()?.checkpoint(parse_session(session_id)?))?;
            println!("{}", pinned.root);
            Ok(0)
        }
        SandboxCommand::Undo { session_id, steps } => {
            let undone = rt.block_on(client()?.undo(parse_session(session_id)?, *steps))?;
            println!("{}", undone.root);
            Ok(0)
        }
        SandboxCommand::Fork {
            session_id,
            count,
            command,
        } => {
            let children =
                rt.block_on(client()?.fork_command(parse_session(session_id)?, *count, command))?;
            let mut code = 0;
            for child in &children {
                let outcome = match (child.exit_code, &child.error) {
                    (Some(0), _) => "ok".to_string(),
                    (Some(exit), _) => {
                        code = 1;
                        format!("exit {exit}")
                    }
                    (None, error) => {
                        code = 1;
                        format!("error: {}", error.as_deref().unwrap_or("unknown"))
                    }
                };
                println!(
                    "{} {} changed={}",
                    child.session_id, outcome, child.changed_files
                );
            }
            Ok(code)
        }
    }
}

/// Chooses where `node` scripts run on this host.
///
/// On Linux the top-level process runs each script in a child process with
/// namespaces, seccomp, and a cgroup memory limit. Elsewhere no runner is installed,
/// so `node` is disabled. `FRONTAL_SANDBOX_UNSAFE_IN_PROCESS_SCRIPTS=1` runs scripts
/// in this process with no containment. It is for development only.
///
/// A worker, and a script child, already run inside an isolated process: the worker
/// process is the boundary, with its own cgroup memory cap. Those run scripts in
/// their own process. A second containment layer inside them would be nested, and
/// the worker's inherited seccomp filter and undelegated cgroup make that impossible.
#[cfg(unix)]
fn install_script_runner(command: &Commands) -> Result<(), SandboxError> {
    if std::env::var_os("FRONTAL_SANDBOX_UNSAFE_IN_PROCESS_SCRIPTS").is_some_and(|v| v == "1") {
        eprintln!("warning: node scripts run in-process without containment");
        return sandbox_runtime::install_in_process_script_runner();
    }
    if matches!(
        command,
        Commands::Worker { .. } | Commands::ScriptRun { .. }
    ) {
        return sandbox_runtime::install_in_process_script_runner();
    }
    if cfg!(target_os = "linux") {
        let program = std::env::current_exe().map_err(|e| {
            SandboxError::internal(format!("locating the script runner binary: {e}"))
        })?;
        let runner = sandbox_worker::ScriptProcess::new(program);
        return sandbox_runtime::install_script_runner(std::sync::Arc::new(runner));
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_script_runner(_command: &Commands) -> Result<(), SandboxError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_fork_takes_the_command_after_a_separator() {
        let cli = Cli::try_parse_from([
            "frontal-sandbox",
            "sandbox",
            "fork",
            "01HZZZZZZZZZZZZZZZZZZZZZZZ",
            "-c",
            "3",
            "--",
            "touch",
            "/r",
        ])
        .unwrap();
        let Commands::Sandbox {
            command: SandboxCommand::Fork { count, command, .. },
        } = cli.command
        else {
            panic!("expected sandbox fork");
        };
        assert_eq!(count, 3);
        assert_eq!(command, ["touch", "/r"]);
    }

    #[test]
    fn sandbox_fork_needs_a_command() {
        assert!(Cli::try_parse_from([
            "frontal-sandbox",
            "sandbox",
            "fork",
            "01HZZZZZZZZZZZZZZZZZZZZZZZ",
        ])
        .is_err());
    }

    #[test]
    fn replay_refuses_a_missing_trace_as_invalid_argument() {
        let replay =
            Cli::try_parse_from(["frontal-sandbox", "replay", "no-such-trace.jsonl"]).unwrap();
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
