//! Runs parsed scripts against a VFS.
//!
//! Command lookup order: function, builtin, then a WASI tool (the registry at
//! `TOOLS_MANIFEST`, then `PATH`, default `/bin`), then exit 127. The shell
//! never runs host binaries. Every command counts against a step budget, so
//! a loop cannot hang the host.

use crate::arith;
use crate::ast::{
    CaseClause, Ch, Command, Connector, ForClause, IfClause, Mode, Param, ParamOp, Piece, Pipeline,
    Redirect, RedirectKind, Script, SimpleCommand, WhileClause, Word,
};
use crate::expand::{
    glob_match, has_glob, push_text, push_values, split_assignment, split_fields, text, trim,
};
use crate::parser;
use sandbox_types::SandboxError;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

/// Where the tool registry lives in the VFS.
pub const TOOLS_MANIFEST: &str = "/etc/tools.toml";

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Starting directory. Must be an absolute path.
    pub cwd: String,
    pub env: HashMap<String, String>,
    /// Commands one `run` may execute, counting loop iterations and function
    /// calls. Exceeding it aborts the run.
    pub max_steps: u64,
    /// Bytes of stdout plus stderr one `run` may produce. Exceeding it aborts the run.
    /// The default matches the trace's per-session cap, so a run that fits here can be
    /// recorded in full.
    pub max_output_bytes: usize,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            cwd: "/".to_string(),
            env: HashMap::new(),
            max_steps: 1_000_000,
            max_output_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Non-local control transfer. `break`, `continue`, and `return` are consumed
/// by the matching loop or function. `Abort` is consumed only by `run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Break(u32),
    Continue(u32),
    Return(i32),
    /// `exit N`: ends the current shell, or the subshell it runs in.
    Exit(i32),
    Abort,
}

/// Input that a compound command reads from, one line at a time. `read` takes
/// lines from it, so `printf 'a\nb\n' | while read x; do ...; done` sees both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Input {
    pub(crate) data: Vec<u8>,
    pub(crate) pos: usize,
}

impl Input {
    pub(crate) fn new(data: Vec<u8>) -> Self {
        Self { data, pos: 0 }
    }

    /// Takes everything not yet read.
    pub(crate) fn take_rest(&mut self) -> Vec<u8> {
        let rest = self.data[self.pos..].to_vec();
        self.pos = self.data.len();
        rest
    }

    /// The next line without its newline, or `None` at the end.
    pub(crate) fn next_line(&mut self) -> Option<(Vec<u8>, bool)> {
        if self.pos >= self.data.len() {
            return None;
        }
        let rest = &self.data[self.pos..];
        let (line, complete) = match rest.iter().position(|&b| b == b'\n') {
            Some(end) => {
                self.pos += end + 1;
                (rest[..end].to_vec(), true)
            }
            None => {
                self.pos = self.data.len();
                (rest.to_vec(), false)
            }
        };
        Some((line, complete))
    }
}

/// Whether a word is quoted at all. A quoted word that expands to nothing is
/// still one empty field. `"$@"` is not counted, because with no parameters it
/// is no field.
fn word_quoted(word: &Word) -> bool {
    // `"$@"` with no parameters is no field, so the quotes around it do not count.
    let has_at = word
        .0
        .iter()
        .any(|piece| matches!(piece, Piece::Param(param) if param.name == "@"));
    word.0.iter().any(|piece| match piece {
        Piece::Ch(ch) => ch.mode != Mode::Bare,
        Piece::Subst { quoted, .. } => *quoted,
        Piece::Param(param) => param.quoted && param.name != "@",
        Piece::Empty => !has_at,
        Piece::Arith(_) => false,
    })
}

/// Where an output descriptor goes once the redirections are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Sink {
    Stdout,
    Stderr,
    /// `/dev/null`: output is discarded.
    Null,
    /// A file. It was created or truncated when the redirection was applied.
    File(String),
}

/// Output that reached the top level of a run, as it becomes available. See
/// [`Shell::run_streamed`] for when a chunk is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChunk {
    /// 1 for stdout, 2 for stderr.
    pub fd: u8,
    pub data: Vec<u8>,
}

/// Receives output chunks as a run produces them. It is called on the run's task,
/// so it must not block.
pub type OutputObserver = Arc<dyn Fn(OutputChunk) + Send + Sync>;

/// Receives a tool's stdout as the tool writes it. The same shape as the runtime's
/// and the WASI host's observers, so one value serves each.
pub(crate) type ToolStdout = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Shell state that persists across calls: working directory, variables,
/// functions, and the last exit status (`$?`).
pub struct Shell {
    fs: Arc<Vfs>,
    cwd: String,
    env: HashMap<String, String>,
    status: i32,
    functions: BTreeMap<String, Command>,
    positional: Vec<String>,
    pub(crate) flow: Option<Flow>,
    /// Stdin for the next command that reads it. A pipeline sets it per stage.
    pub(crate) pending_stdin: Option<Vec<u8>>,
    steps: u64,
    max_steps: u64,
    /// Compound commands open around the command being run. See `run_command`.
    depth: usize,
    output_used: usize,
    max_output_bytes: usize,
    /// State of the generator behind `$RANDOM`. Not for secrets.
    random: std::sync::atomic::AtomicU64,
    /// Input a compound command is reading. See `Input`.
    pub(crate) input: Option<Input>,
    /// Bytes of stdout a tool streamed while the simple command being run was
    /// running. `None` when that command's output is not streamed. See `tool_stdout`.
    stream_count: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    /// The status of the last command substitution, for an assignment that
    /// has no command, such as `x=$(false)`.
    subst_status: Option<i32>,
    /// An expansion error, such as a division by zero. It stops the command.
    expansion_error: Option<String>,
    /// Receives top-level output as it is produced. `None` buffers it in the result only.
    observer: Option<OutputObserver>,
    /// Set while the output of the command being run is consumed by something
    /// else: a pipe, a redirection, or a substitution. Such output is not streamed.
    capturing: bool,
}

fn literal_word(s: &str) -> Word {
    Word(
        s.chars()
            .map(|c| {
                Piece::Ch(Ch {
                    c,
                    mode: crate::ast::Mode::Literal,
                })
            })
            .collect(),
    )
}

fn merge(into: &mut ExecutionResult, from: ExecutionResult) {
    into.stdout.extend(from.stdout);
    into.stderr.extend(from.stderr);
}

impl Shell {
    pub fn new(fs: Arc<Vfs>, config: ShellConfig) -> Self {
        Self {
            fs,
            cwd: config.cwd,
            env: config.env,
            status: 0,
            functions: BTreeMap::new(),
            positional: Vec::new(),
            flow: None,
            pending_stdin: None,
            input: None,
            subst_status: None,
            expansion_error: None,
            observer: None,
            capturing: false,
            stream_count: None,
            steps: 0,
            depth: 0,
            max_steps: config.max_steps,
            output_used: 0,
            max_output_bytes: config.max_output_bytes,
            random: std::sync::atomic::AtomicU64::new(random_seed()),
        }
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn env(&self) -> &HashMap<String, String> {
        &self.env
    }

    pub fn set_env(&mut self, key: String, value: String) {
        self.env.insert(key, value);
    }

    pub(crate) fn unset_env(&mut self, name: &str) {
        self.env.remove(name);
    }

    /// Metadata for a path, or `None` if it does not exist.
    pub(crate) async fn stat_path(&self, path: &str) -> Option<sandbox_vfs::FileStat> {
        self.fs.stat(Path::new(&self.resolve(path))).await.ok()
    }

    /// Exit status of the last command. This is `$?`.
    pub(crate) fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// Whether a command reads its stdin. Builtins such as `echo` do not, so they
    /// leave the input of an enclosing `while read` loop alone. Functions do not
    /// read it unless their body does. External tools always may.
    fn reads_input(&self, name: &str) -> bool {
        if self.has_function(name) {
            return false;
        }
        !crate::builtins::is_builtin(name)
            || matches!(
                name,
                "cat" | "head" | "tail" | "wc" | "grep" | "node" | "wasm"
            )
    }

    pub fn status(&self) -> i32 {
        self.status
    }

    /// Changes directory. Fails unless `dir` names an existing directory.
    pub async fn chdir(&mut self, dir: &str) -> Result<(), SandboxError> {
        let path = self.resolve(dir);
        match self.fs.stat(Path::new(&path)).await {
            Ok(stat) if stat.is_dir => {
                self.cwd = path;
                Ok(())
            }
            Ok(_) => Err(SandboxError::invalid_argument(format!(
                "not a directory: {dir}"
            ))),
            Err(e) => Err(SandboxError::not_found(format!("{dir}: {}", describe(&e)))),
        }
    }

    /// Parses and runs a script. Syntax errors are returned as `Err`; command
    /// failures are reported through the exit code and stderr.
    pub async fn run(&mut self, script: &str) -> Result<ExecutionResult, SandboxError> {
        self.run_streamed(script, None).await
    }

    /// Like [`Shell::run`], and also reports output to `observer` as it is
    /// produced. A command's output is reported when the command finishes, so a
    /// command that writes a lot and then stops does not stream part of it early.
    /// Output inside a pipe, a redirection, or a substitution is not reported,
    /// because the shell consumes it. It is still in the result. The chunks
    /// concatenate to the result's output, except for output that the shell
    /// itself reports only in the result (see `run_case`).
    pub async fn run_streamed(
        &mut self,
        script: &str,
        observer: Option<OutputObserver>,
    ) -> Result<ExecutionResult, SandboxError> {
        let script: Script = parser::parse(script)?;
        self.steps = 0;
        self.output_used = 0;
        self.observer = observer;
        // A run cancelled inside a pipe or a substitution may have left this set.
        self.capturing = false;
        let result = self.run_script(&script).await;
        self.observer = None;
        Ok(self.finish(result))
    }

    /// Drops the observer. A run that the caller cancelled never reaches the end
    /// of its own, so its caller clears the observer through this.
    pub fn detach_observer(&mut self) {
        self.observer = None;
    }

    /// Runs `argv` as one command, with `stdin` as its input. Arguments are
    /// passed through unexpanded.
    pub async fn run_argv(
        &mut self,
        argv: &[String],
        stdin: &[u8],
    ) -> Result<ExecutionResult, SandboxError> {
        self.run_argv_streamed(argv, stdin, None).await
    }

    /// Like [`Shell::run_argv`], and reports the command's output to `observer`
    /// when the command finishes. One command has one chunk per stream.
    pub async fn run_argv_streamed(
        &mut self,
        argv: &[String],
        stdin: &[u8],
        observer: Option<OutputObserver>,
    ) -> Result<ExecutionResult, SandboxError> {
        if argv.is_empty() {
            return Err(SandboxError::invalid_argument("argv is empty"));
        }
        let command = SimpleCommand {
            words: argv.iter().map(|a| literal_word(a)).collect(),
            redirects: Vec::new(),
        };
        self.steps = 0;
        self.output_used = 0;
        self.pending_stdin = Some(stdin.to_vec());
        self.observer = observer;
        self.capturing = false;
        let out = self.run_simple(&command).await;
        self.observer = None;
        self.pending_stdin = None;
        let out = self.finish(out);
        self.status = out.exit_code;
        Ok(out)
    }

    /// Ends a run. A run that hit a limit fails, even when the command that was
    /// running last succeeded, as a substitution that was cut short does.
    fn finish(&mut self, mut result: ExecutionResult) -> ExecutionResult {
        match self.flow.take() {
            Some(Flow::Exit(code)) => result.exit_code = code,
            // A run that hit a limit fails, even when the command running then succeeded.
            Some(Flow::Abort) if result.exit_code == 0 => result.exit_code = 1,
            _ => {}
        }
        result
    }

    /// A copy of the shell state, for subshells and command substitutions.
    /// Changes made to the copy do not reach this shell.
    /// Takes back what a forked child used. A limit the child hit aborts this run
    /// too, at the point it was hit, instead of waiting for this run's next command.
    fn absorb(&mut self, child: &Shell) {
        self.steps = child.steps;
        self.output_used = child.output_used;
        // A subshell that reads the input consumes it for the parent too.
        if let (Some(mine), Some(theirs)) = (self.input.as_mut(), child.input.as_ref()) {
            mine.pos = theirs.pos;
        }
        // A limit the child hit aborts this run. `exit` ends only the child.
        if child.flow == Some(Flow::Abort) {
            self.flow = Some(Flow::Abort);
        }
        // The child's $RANDOM sequence continues here, so repeated subshells do not repeat.
        self.random.store(
            child.random.load(std::sync::atomic::Ordering::Relaxed),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    fn fork(&self) -> Shell {
        Shell {
            fs: self.fs.clone(),
            cwd: self.cwd.clone(),
            env: self.env.clone(),
            status: self.status,
            functions: self.functions.clone(),
            positional: self.positional.clone(),
            flow: None,
            pending_stdin: None,
            input: self.input.clone(),
            subst_status: None,
            expansion_error: None,
            observer: self.observer.clone(),
            capturing: self.capturing,
            stream_count: None,
            steps: self.steps,
            depth: self.depth,
            max_steps: self.max_steps,
            output_used: self.output_used,
            max_output_bytes: self.max_output_bytes,
            random: std::sync::atomic::AtomicU64::new(
                self.random.load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }

    fn run_script<'a>(&'a mut self, script: &'a Script) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            let mut result = ExecutionResult::default();
            let mut previous = Connector::Seq;
            let mut items = script.items.iter();
            // Once a flow such as `break` is pending, the rest of the script does not run.
            while let Some(item) = items.next().filter(|_| self.flow.is_none()) {
                let should_run = match previous {
                    Connector::Seq => true,
                    Connector::And => self.status == 0,
                    Connector::Or => self.status != 0,
                };
                // A skipped item leaves the status unchanged, as in POSIX sh.
                if should_run {
                    let out = self.run_pipeline(&item.pipeline).await;
                    self.status = out.exit_code;
                    merge(&mut result, out);
                }
                previous = item.connector;
            }
            result.exit_code = self.status;
            result
        })
    }

    fn run_pipeline<'a>(&'a mut self, pipeline: &'a Pipeline) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            // The first stage gets the stdin the pipeline was given, if any. Later
            // stages get the output of the stage before. `None` means a stage has
            // no stdin of its own, so it reads the input of an enclosing compound.
            let mut input: Option<Vec<u8>> = self.pending_stdin.take();
            let mut stderr = Vec::new();
            let mut exit_code = 0;
            let last = pipeline.stages.len().saturating_sub(1);
            for (index, stage) in pipeline.stages.iter().enumerate() {
                self.pending_stdin = input.take();
                // Only the last stage's output reaches the top level. The others feed the next stage.
                let outer = self.capturing;
                self.capturing = outer || index < last;
                let out = self.run_command(stage).await;
                self.capturing = outer;
                self.pending_stdin = None;
                stderr.extend(out.stderr);
                exit_code = out.exit_code;
                input = Some(out.stdout);
                self.status = exit_code;
            }
            let input = input.unwrap_or_default();
            // `!` inverts the status of the pipeline. It does not change its output.
            if pipeline.negated {
                exit_code = i32::from(exit_code == 0);
                self.status = exit_code;
            }
            ExecutionResult {
                exit_code,
                stdout: input,
                stderr,
            }
        })
    }

    fn run_command<'a>(&'a mut self, command: &'a Command) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            self.steps += 1;
            if self.steps > self.max_steps {
                self.flow = Some(Flow::Abort);
                return failure_with("sandbox: step limit exceeded".to_string(), 1);
            }
            // Every recursive path runs through here: function calls, subshells,
            // substitutions, and loop bodies. The step budget alone is too late, because
            // a recursive call overflows the stack before it runs out (ADR 0018).
            if self.depth >= parser::MAX_COMMAND_DEPTH {
                self.flow = Some(Flow::Abort);
                return failure_with(
                    format!(
                        "sandbox: commands nested deeper than {} levels",
                        parser::MAX_COMMAND_DEPTH
                    ),
                    1,
                );
            }
            self.depth += 1;
            let out = match command {
                Command::Simple(simple) => self.run_simple(simple).await,
                Command::Function(def) => {
                    self.functions.insert(def.name.clone(), (*def.body).clone());
                    ExecutionResult::default()
                }
                Command::Redirected { command, redirects } => {
                    let stdin = self.pending_stdin.take();
                    self.run_redirected(command, redirects, stdin).await
                }
                compound => {
                    let stdin = self.pending_stdin.take();
                    self.run_compound(compound, stdin).await
                }
            };
            self.depth -= 1;
            out
        })
    }

    /// Runs a compound command. Its stdin, if it was given one, becomes the
    /// input its commands read, and is restored afterwards.
    async fn run_compound(&mut self, command: &Command, stdin: Option<Vec<u8>>) -> ExecutionResult {
        let subshell_stdin = stdin.clone();
        let saved = stdin.map(|data| self.input.replace(Input::new(data)));
        let out = match command {
            Command::If(clause) => self.run_if(clause).await,
            Command::While(clause) => self.run_while(clause).await,
            Command::For(clause) => self.run_for(clause).await,
            Command::Case(clause) => self.run_case(clause).await,
            Command::Group(body) => self.run_script(body).await,
            Command::Subshell(body) => {
                let mut child = self.fork();
                // The subshell's first command reads the stdin it was given.
                child.pending_stdin = subshell_stdin;
                let out = child.run_script(body).await;
                self.absorb(&child);
                out
            }
            Command::Simple(_) | Command::Function(_) | Command::Redirected { .. } => {
                unreachable!("run_command dispatches these before run_compound")
            }
        };
        if let Some(previous) = saved {
            self.input = previous;
        }
        out
    }

    async fn run_if(&mut self, clause: &IfClause) -> ExecutionResult {
        let mut out = ExecutionResult::default();
        // The first branch whose condition succeeds is chosen. A pending flow ends the search.
        let mut chosen = None;
        let mut branches = clause.branches.iter();
        while let Some((condition, body)) = branches
            .next()
            .filter(|_| self.flow.is_none() && chosen.is_none())
        {
            merge(&mut out, self.run_script(condition).await);
            if self.flow.is_none() && self.status == 0 {
                chosen = Some(body);
            }
        }
        if let Some(body) = chosen {
            merge(&mut out, self.run_script(body).await);
            out.exit_code = self.status;
            return out;
        }
        match &clause.otherwise {
            Some(body) if self.flow.is_none() => merge(&mut out, self.run_script(body).await),
            // No branch ran and there is no else: the status is 0.
            _ => self.status = 0,
        }
        out.exit_code = self.status;
        out
    }

    async fn run_while(&mut self, clause: &WhileClause) -> ExecutionResult {
        let mut out = ExecutionResult::default();
        let mut status = 0;
        let mut stopped = false;
        while !stopped && self.while_condition_holds(clause, &mut out).await {
            let body = self.run_script(&clause.body).await;
            status = body.exit_code;
            merge(&mut out, body);
            stopped = self.loop_should_stop();
        }
        out.exit_code = status;
        out
    }

    /// Runs the condition of a `while` or `until` loop and says whether its body runs
    /// next. A pending flow, or a status that `until` does not allow, ends the loop.
    async fn while_condition_holds(
        &mut self,
        clause: &WhileClause,
        out: &mut ExecutionResult,
    ) -> bool {
        merge(out, self.run_script(&clause.condition).await);
        self.flow.is_none() && (self.status == 0) != clause.until
    }

    async fn run_for(&mut self, clause: &ForClause) -> ExecutionResult {
        let mut out = ExecutionResult::default();
        let mut values = Vec::new();
        for word in &clause.words {
            let (fields, expansion_stderr) = self.expand_word(word).await;
            out.stderr.extend(expansion_stderr);
            values.extend(fields);
        }
        // A loop with no iterations has status 0. Otherwise the status is that of
        // its last body command.
        let mut status = 0;
        let mut stopped = false;
        let mut values = values.into_iter();
        while let Some(value) = values.next().filter(|_| !stopped) {
            self.env.insert(clause.variable.clone(), value);
            let body = self.run_script(&clause.body).await;
            status = body.exit_code;
            merge(&mut out, body);
            stopped = self.loop_should_stop();
        }
        out.exit_code = status;
        out
    }

    /// Consumes a `break` or `continue` aimed at the current loop. Returns true
    /// when the loop must stop: a `break`, or any flow that leaves the loop.
    fn loop_should_stop(&mut self) -> bool {
        match self.flow {
            None => false,
            Some(Flow::Continue(n)) if n <= 1 => {
                self.flow = None;
                false
            }
            Some(Flow::Continue(n)) => {
                self.flow = Some(Flow::Continue(n - 1));
                true
            }
            Some(Flow::Break(n)) if n <= 1 => {
                self.flow = None;
                true
            }
            Some(Flow::Break(n)) => {
                self.flow = Some(Flow::Break(n - 1));
                true
            }
            Some(Flow::Return(_) | Flow::Exit(_) | Flow::Abort) => true,
        }
    }

    async fn run_simple(&mut self, command: &SimpleCommand) -> ExecutionResult {
        let out = self.run_simple_inner(command).await;
        // Stdout a tool streamed while it ran is the start of this command's stdout.
        // Only the rest is left to report.
        let streamed = self
            .stream_count
            .take()
            .map_or(0, |count| count.load(std::sync::atomic::Ordering::SeqCst));
        self.emit_after(&out, streamed);
        out
    }

    /// Reports a finished command's output to the observer, unless the output is
    /// being captured by a pipe, a redirection, or a substitution.
    fn emit(&self, out: &ExecutionResult) {
        self.emit_after(out, 0);
    }

    /// Reports `out`, except its first `streamed` bytes of stdout, which the observer
    /// already has.
    fn emit_after(&self, out: &ExecutionResult, streamed: usize) {
        if self.capturing {
            return;
        }
        let Some(observer) = &self.observer else {
            return;
        };
        let stdout = &out.stdout[streamed.min(out.stdout.len())..];
        if !stdout.is_empty() {
            observer(OutputChunk {
                fd: 1,
                data: stdout.to_vec(),
            });
        }
        if !out.stderr.is_empty() {
            observer(OutputChunk {
                fd: 2,
                data: out.stderr.clone(),
            });
        }
    }

    /// Where a tool's stdout goes as the tool writes it, or `None` when the output
    /// of the command being run is not streamed (see `stream_count`).
    pub(crate) fn tool_stdout(&self) -> Option<ToolStdout> {
        let count = self.stream_count.clone()?;
        // A weak reference: a tool can outlive the command that started it, because
        // a runtime keeps running after its caller gave up. The observer must not be
        // kept alive by that, or the stream of a timed-out command would never end.
        let observer = Arc::downgrade(self.observer.as_ref()?);
        Some(Arc::new(move |bytes: &[u8]| {
            let Some(observer) = observer.upgrade() else {
                return;
            };
            count.fetch_add(bytes.len(), std::sync::atomic::Ordering::SeqCst);
            observer(OutputChunk {
                fd: 1,
                data: bytes.to_vec(),
            });
        }))
    }

    async fn run_simple_inner(&mut self, command: &SimpleCommand) -> ExecutionResult {
        let mut stderr = Vec::new();
        let pending_stdin = self.pending_stdin.take();

        // Leading `NAME=value` words are assignments. The rest is the command.
        self.subst_status = None;
        let mut assignments = Vec::new();
        let mut index = 0;
        while let Some((name, value)) = command.words.get(index).and_then(split_assignment) {
            let (chars, subst_err) = self.expand_chars(&value).await;
            stderr.extend(subst_err);
            assignments.push((name, text(&chars)));
            index += 1;
        }
        // An assignment that runs a substitution has the substitution's status.
        let assignment_status = self.subst_status.take();

        self.expansion_error = None;
        let mut argv = Vec::new();
        for word in &command.words[index..] {
            let (fields, subst_err) = self.expand_word(word).await;
            stderr.extend(subst_err);
            argv.extend(fields);
        }
        if let Some(message) = self.expansion_error.take() {
            // As in bash, a failed expansion ends the run, not just this command.
            self.flow = Some(Flow::Abort);
            stderr.extend(format!("sandbox: {message}\n").into_bytes());
            return ExecutionResult {
                exit_code: 1,
                stdout: Vec::new(),
                stderr,
            };
        }

        let mut redirects = Vec::new();
        for redirect in &command.redirects {
            let (chars, subst_err) = self.expand_chars(&redirect.target).await;
            stderr.extend(subst_err);
            redirects.push((redirect.fd, redirect.kind, text(&chars)));
        }

        if argv.is_empty() {
            // Assignments alone persist in the shell. Redirections with no
            // command only create or truncate their target files.
            for (name, value) in assignments {
                self.env.insert(name, value);
            }
            if let Err(message) = self.open_sinks(&redirects).await {
                stderr.extend(message.into_bytes());
            }
            let exit_code = if stderr.is_empty() {
                assignment_status.unwrap_or(0)
            } else {
                1
            };
            return ExecutionResult {
                exit_code,
                stdout: Vec::new(),
                stderr,
            };
        }

        // Input redirections load stdin before the command runs. The last one wins.
        let mut stdin_override = None;
        for (_, kind, target) in &redirects {
            if *kind == RedirectKind::Input {
                match self.read_file(target).await {
                    Ok(data) => stdin_override = Some(data),
                    Err(e) => {
                        stderr.extend(format!("{target}: {}\n", describe(&e)).into_bytes());
                        return ExecutionResult {
                            exit_code: 1,
                            stdout: Vec::new(),
                            stderr,
                        };
                    }
                }
            }
        }
        let sinks = match self.open_sinks(&redirects).await {
            Ok(sinks) => sinks,
            Err(message) => {
                stderr.extend(message.into_bytes());
                return ExecutionResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr,
                };
            }
        };

        // Prefix assignments apply to this command only.
        let saved: Vec<(String, Option<String>)> = assignments
            .iter()
            .map(|(name, _)| (name.clone(), self.env.get(name).cloned()))
            .collect();
        for (name, value) in assignments {
            self.env.insert(name, value);
        }

        // Without stdin of its own, a command reads what is left of the input the
        // enclosing compound command is reading, as `cat` does in `{ cat; }`. `read`
        // takes one line of it instead, so it is left alone here.
        // An input redirection replaces both, and it leaves the input unread.
        let stdin = match (stdin_override, pending_stdin) {
            (Some(data), _) => data,
            (None, Some(data)) => data,
            (None, None) if argv[0] == "read" => Vec::new(),
            (None, None) if self.reads_input(&argv[0]) => self
                .input
                .as_mut()
                .map(Input::take_rest)
                .unwrap_or_default(),
            (None, None) => Vec::new(),
        };
        // A tool streams its stdout only for a command whose output is plain: no
        // redirection, and not captured by a pipe or a substitution.
        let streams = redirects.is_empty() && !self.capturing && matches!(sinks.0, Sink::Stdout);
        self.stream_count =
            streams.then(|| std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let out = self.run_argv_inner(&argv, stdin).await;

        for (name, previous) in saved.into_iter().rev() {
            match previous {
                Some(value) => self.env.insert(name, value),
                None => self.env.remove(&name),
            };
        }

        let routed = self.route(out, &sinks).await;
        stderr.extend(routed.stderr);
        let mut result = ExecutionResult {
            exit_code: routed.exit_code,
            stdout: routed.stdout,
            stderr,
        };
        self.charge_output(&mut result);
        result
    }

    /// Applies the redirections of one command, in order, to its two output
    /// descriptors. Output files are created or truncated now. Returns where
    /// descriptors 1 and 2 go, or the error that stops the command.
    async fn open_sinks(
        &self,
        redirects: &[(u8, RedirectKind, String)],
    ) -> Result<(Sink, Sink), String> {
        let mut sinks = [Sink::Null, Sink::Stdout, Sink::Stderr];
        for (fd, kind, target) in redirects.iter().filter(|(fd, _, _)| *fd != 0) {
            let fd = usize::from(*fd);
            match kind {
                RedirectKind::Input => {}
                RedirectKind::Output | RedirectKind::Append => {
                    sinks[fd] = if target == "/dev/null" {
                        Sink::Null
                    } else {
                        let append = *kind == RedirectKind::Append;
                        self.write_file(target, &[], append)
                            .await
                            .map_err(|e| format!("{target}: {}\n", describe(&e)))?;
                        Sink::File(target.clone())
                    };
                }
                RedirectKind::Dup => {
                    let source = match target.as_str() {
                        "1" => sinks[1].clone(),
                        "2" => sinks[2].clone(),
                        _ => return Err(format!("sandbox: {target}: ambiguous redirect\n")),
                    };
                    sinks[fd] = source;
                }
            }
        }
        Ok((sinks[1].clone(), sinks[2].clone()))
    }

    /// Sends a command's output to where its descriptors point. Output to a file
    /// is appended, because the file was created when the redirection was applied.
    async fn route(&mut self, out: ExecutionResult, sinks: &(Sink, Sink)) -> ExecutionResult {
        let mut result = ExecutionResult {
            exit_code: out.exit_code,
            ..ExecutionResult::default()
        };
        for (data, sink) in [(out.stdout, &sinks.0), (out.stderr, &sinks.1)]
            .into_iter()
            .filter(|(data, _)| !data.is_empty())
        {
            match sink {
                Sink::Stdout => result.stdout.extend(data),
                Sink::Stderr => result.stderr.extend(data),
                Sink::Null => {}
                Sink::File(path) => {
                    if let Err(e) = self.write_file(path, &data, true).await {
                        result
                            .stderr
                            .extend(format!("{path}: {}\n", describe(&e)).into_bytes());
                        result.exit_code = 1;
                    }
                }
            }
        }
        result
    }

    /// A compound command with redirections after it. Its input redirection
    /// sets the input it reads, and its output goes where the redirections say.
    async fn run_redirected(
        &mut self,
        command: &Command,
        redirects: &[Redirect],
        stdin: Option<Vec<u8>>,
    ) -> ExecutionResult {
        let out = self.run_redirected_inner(command, redirects, stdin).await;
        self.emit(&out);
        out
    }

    async fn run_redirected_inner(
        &mut self,
        command: &Command,
        redirects: &[Redirect],
        stdin: Option<Vec<u8>>,
    ) -> ExecutionResult {
        let mut stderr = Vec::new();
        let mut targets = Vec::new();
        for redirect in redirects {
            let (chars, subst_err) = self.expand_chars(&redirect.target).await;
            stderr.extend(subst_err);
            targets.push((redirect.fd, redirect.kind, text(&chars)));
        }
        let mut input = stdin;
        for (_, kind, target) in &targets {
            if *kind == RedirectKind::Input {
                match self.read_file(target).await {
                    Ok(data) => input = Some(data),
                    Err(e) => {
                        stderr.extend(format!("{target}: {}\n", describe(&e)).into_bytes());
                        return ExecutionResult {
                            exit_code: 1,
                            stdout: Vec::new(),
                            stderr,
                        };
                    }
                }
            }
        }
        let sinks = match self.open_sinks(&targets).await {
            Ok(sinks) => sinks,
            Err(message) => {
                stderr.extend(message.into_bytes());
                return ExecutionResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr,
                };
            }
        };
        // The compound's commands write to the redirected descriptors, so their
        // output is not streamed on its own. The routed result is emitted once.
        let outer = std::mem::replace(&mut self.capturing, true);
        let out = self.run_compound(command, input).await;
        self.capturing = outer;
        let routed = self.route(out, &sinks).await;
        stderr.extend(routed.stderr);
        ExecutionResult {
            exit_code: routed.exit_code,
            stdout: routed.stdout,
            stderr,
        }
    }

    /// Runs the first arm whose pattern matches the subject. Patterns are
    /// globs, and quoted characters in them match literally. With no match the
    /// status is 0, as in POSIX sh.
    async fn run_case(&mut self, clause: &CaseClause) -> ExecutionResult {
        let (subject_chars, mut stderr) = self.expand_chars(&clause.subject).await;
        let subject: Vec<char> = text(&subject_chars).chars().collect();
        for arm in &clause.arms {
            for pattern in &arm.patterns {
                let (pattern_chars, subst_err) = self.expand_chars(pattern).await;
                stderr.extend(subst_err);
                if glob_match(&pattern_chars, &subject) {
                    let body = self.run_script(&arm.body).await;
                    return ExecutionResult {
                        exit_code: body.exit_code,
                        stdout: body.stdout,
                        stderr: [stderr, body.stderr].concat(),
                    };
                }
            }
        }
        ExecutionResult {
            exit_code: 0,
            stdout: Vec::new(),
            stderr,
        }
    }

    /// Counts a command's output against the run's budget. Going over stops
    /// the run, and the output that does not fit is dropped.
    fn charge_output(&mut self, result: &mut ExecutionResult) {
        let size = result.stdout.len() + result.stderr.len();
        self.output_used += size;
        if self.output_used > self.max_output_bytes {
            let room = self
                .max_output_bytes
                .saturating_sub(self.output_used - size);
            result.stdout.truncate(room.min(result.stdout.len()));
            result.stderr.clear();
            result
                .stderr
                .extend_from_slice(b"sandbox: output limit exceeded\n");
            result.exit_code = 1;
            self.flow = Some(Flow::Abort);
        }
    }

    /// Runs an expanded command: function, builtin, tool, then 127.
    async fn run_argv_inner(&mut self, argv: &[String], stdin: Vec<u8>) -> ExecutionResult {
        if let Some(body) = self.functions.get(&argv[0]).cloned() {
            // The call is one command, so its body's output is reported once, when the
            // call finishes, and not again through the body's own commands.
            let outer = std::mem::replace(&mut self.capturing, true);
            let out = self.call_function(body, &argv[1..], stdin).await;
            self.capturing = outer;
            return out;
        }
        if let Some(out) = self.builtin(argv, stdin.clone()).await {
            return out;
        }
        let name = &argv[0];
        let registry = match self.registry().await {
            Ok(registry) => registry,
            Err(e) => return failure_with(format!("{name}: {}", e.message), 126),
        };

        // A registered name wins. Otherwise a name is searched on PATH, or a
        // path with a slash is used as given.
        let (path, spec) = match registry.find(name) {
            Some(spec) => (self.resolve(&spec.path), Some(spec.clone())),
            None => match self.find_tool(name).await {
                Some(path) => (path, None),
                None => return failure_with(format!("sandbox: {name}: command not found"), 127),
            },
        };

        let module = match self.read_file(&path).await {
            Ok(module) => module,
            Err(e) => return failure_with(format!("{name}: {}", describe(&e)), 126),
        };
        let limits = match &spec {
            Some(spec) => {
                if let Err(e) = spec.verify(&module) {
                    return failure_with(format!("{name}: {}", e.message), 126);
                }
                spec.limits()
            }
            None => sandbox_wasm::WasiLimits::default(),
        };
        match sandbox_wasm::run_module_with_mount(
            module,
            argv.to_vec(),
            stdin,
            limits,
            Some(self.mount()),
        )
        .await
        {
            Ok(out) => ExecutionResult {
                exit_code: out.exit_code,
                stdout: out.stdout,
                stderr: out.stderr,
            },
            Err(e) => failure_with(format!("{name}: cannot execute: {}", e.message), 126),
        }
    }

    /// Calls a function with its own positional parameters. A `return` ends
    /// the call with its status.
    async fn call_function(
        &mut self,
        body: Command,
        args: &[String],
        stdin: Vec<u8>,
    ) -> ExecutionResult {
        let saved = std::mem::replace(&mut self.positional, args.to_vec());
        self.pending_stdin = Some(stdin);
        let mut out = self.run_command(&body).await;
        self.pending_stdin = None;
        if let Some(Flow::Return(code)) = self.flow {
            self.flow = None;
            self.status = code;
            out.exit_code = code;
        }
        self.positional = saved;
        out
    }

    /// The tool registry at `TOOLS_MANIFEST`. A missing manifest is an empty
    /// registry. A malformed one is an error, so tools fail closed.
    pub(crate) async fn registry(&self) -> Result<sandbox_wasm::Registry, SandboxError> {
        match self.read_file(TOOLS_MANIFEST).await {
            Ok(bytes) => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| SandboxError::invalid_argument("tools manifest is not UTF-8"))?;
                sandbox_wasm::Registry::parse(&text)
            }
            Err(sandbox_vfs::VfsError::NotFound(_)) => Ok(sandbox_wasm::Registry::default()),
            Err(e) => Err(SandboxError::internal(format!(
                "{TOOLS_MANIFEST}: {}",
                describe(&e)
            ))),
        }
    }

    /// Finds a tool on PATH: a path with a slash, or a name searched on PATH.
    async fn find_tool(&self, name: &str) -> Option<String> {
        let candidates: Vec<String> = if name.contains('/') {
            vec![self.resolve(name)]
        } else {
            let path = self.env.get("PATH").map_or("/bin", String::as_str);
            path.split(':')
                .filter(|dir| !dir.is_empty())
                .map(|dir| self.resolve(&format!("{dir}/{name}")))
                .collect()
        };
        for candidate in candidates {
            if let Ok(stat) = self.fs.stat(Path::new(&candidate)).await {
                if stat.is_file {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Expands one word into the fields it produces: variables, substitutions,
    /// and globs. Unquoted words that expand to nothing produce no field.
    /// Expands a word into fields: splits unquoted expansions on `IFS`, then
    /// globs each field that has unquoted glob characters.
    async fn expand_word(&mut self, word: &Word) -> (Vec<String>, Vec<u8>) {
        let (chars, stderr) = self.expand_chars(word).await;
        let mut words = Vec::new();
        for field in split_fields(&chars, word_quoted(word)) {
            // A pattern that matches nothing stays as it was written.
            let matches = if has_glob(&field) {
                self.glob(&field).await
            } else {
                Vec::new()
            };
            if matches.is_empty() {
                words.push(text(&field));
            } else {
                words.extend(matches);
            }
        }
        (words, stderr)
    }

    /// Expands the pieces of a word to characters, keeping each one's quoting.
    async fn expand_chars(&mut self, word: &Word) -> (Vec<Ch>, Vec<u8>) {
        let mut stderr = Vec::new();
        let mut out: Vec<Ch> = Vec::new();
        for piece in &word.0 {
            match piece {
                Piece::Ch(ch) => out.push(*ch),
                Piece::Subst { script, quoted } => {
                    let (output, sub_stderr) = self.substitute(script, *quoted).await;
                    stderr.extend(sub_stderr);
                    out.extend(output);
                }
                Piece::Param(param) => {
                    let (chars, param_stderr) = self.expand_param(param).await;
                    stderr.extend(param_stderr);
                    out.extend(chars);
                }
                Piece::Arith(expr) => {
                    let (chars, expr_stderr) = self.expand_arith_word(expr).await;
                    stderr.extend(expr_stderr);
                    match self.arithmetic(&text(&chars)) {
                        Ok(value) => push_text(&mut out, &value.to_string(), Mode::Expanded),
                        Err(message) => self.expansion_error = Some(message),
                    }
                }
                Piece::Empty => {}
            }
        }
        (out, stderr)
    }

    /// Runs a command substitution in a copy of the shell and returns its
    /// output as literal characters, minus trailing newlines.
    /// Runs a command substitution in a copy of the shell and returns its output
    /// with trailing newlines removed. Its status is kept for `x=$(cmd)`.
    async fn substitute(&mut self, script: &Script, quoted: bool) -> (Vec<Ch>, Vec<u8>) {
        let mut child = self.fork();
        // A substitution's output becomes text in a word, so it is never streamed.
        child.observer = None;
        let out = child.run_script(script).await;
        self.absorb(&child);
        self.subst_status = Some(out.exit_code);
        let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
        while output.ends_with('\n') {
            output.pop();
        }
        let mode = if quoted { Mode::Double } else { Mode::Expanded };
        let mut chars = Vec::new();
        push_text(&mut chars, &output, mode);
        (chars, out.stderr)
    }

    /// Expands `${...}`: a value, its length, a default, or a trimmed value.
    // The expression of `$((...))` is expanded before it is evaluated. Boxed,
    // because the expansion calls back into this module.
    fn expand_arith_word<'a>(&'a mut self, word: &'a Word) -> BoxFuture<'a, (Vec<Ch>, Vec<u8>)> {
        Box::pin(async move { self.expand_chars(word).await })
    }

    // Boxed, because a default word or pattern expands with `expand_chars`, which
    // calls this function again.
    fn expand_param<'a>(&'a mut self, param: &'a Param) -> BoxFuture<'a, (Vec<Ch>, Vec<u8>)> {
        Box::pin(async move { self.expand_param_inner(param).await })
    }

    async fn expand_param_inner(&mut self, param: &Param) -> (Vec<Ch>, Vec<u8>) {
        let mode = if param.quoted {
            Mode::Double
        } else {
            Mode::Expanded
        };
        let mut out = Vec::new();
        let mut stderr = Vec::new();
        let value = self.param_scalar(&param.name);
        match &param.op {
            ParamOp::Value if param.name == "@" => push_values(&mut out, &self.positional, mode),
            ParamOp::Value => push_text(&mut out, value.as_deref().unwrap_or(""), mode),
            ParamOp::Length => {
                let length = value.as_deref().unwrap_or("").chars().count();
                push_text(&mut out, &length.to_string(), mode);
            }
            ParamOp::Default { colon, word } => {
                let missing = match &value {
                    None => true,
                    Some(text) => *colon && text.is_empty(),
                };
                if missing {
                    // The default word is expanded with the quoting of the parameter.
                    let (chars, word_stderr) = self.expand_chars(word).await;
                    stderr.extend(word_stderr);
                    out.extend(chars.into_iter().map(|ch| match ch.mode {
                        Mode::Bare => Ch { mode, ..ch },
                        _ => ch,
                    }));
                } else {
                    push_text(&mut out, value.as_deref().unwrap_or(""), mode);
                }
            }
            ParamOp::Indirect { default } => {
                // The value of the variable named by `value`.
                let target = value.as_deref().and_then(|name| self.param_scalar(name));
                let missing = match (&target, default) {
                    (None, _) => true,
                    (Some(text), Some((colon, _))) => *colon && text.is_empty(),
                    (Some(_), None) => false,
                };
                match (missing, default) {
                    (true, Some((_, word))) => {
                        let (chars, word_stderr) = self.expand_chars(word).await;
                        stderr.extend(word_stderr);
                        out.extend(chars.into_iter().map(|ch| match ch.mode {
                            Mode::Bare => Ch { mode, ..ch },
                            _ => ch,
                        }));
                    }
                    (true, None) => {}
                    (false, _) => push_text(&mut out, target.as_deref().unwrap_or(""), mode),
                }
            }
            ParamOp::Suffix { longest, pattern } | ParamOp::Prefix { longest, pattern } => {
                let prefix = matches!(param.op, ParamOp::Prefix { .. });
                let (pattern_chars, pattern_stderr) = self.expand_chars(pattern).await;
                stderr.extend(pattern_stderr);
                let chars: Vec<char> = value.as_deref().unwrap_or("").chars().collect();
                let trimmed = trim(&chars, &pattern_chars, prefix, *longest);
                push_text(&mut out, &trimmed, mode);
            }
        }
        (out, stderr)
    }

    /// The value of a parameter, or `None` if it is unset. `$@` and `$*` join
    /// the positional parameters with spaces here. `$@` keeps them apart in
    /// `expand_param`.
    fn param_scalar(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(self.status.to_string()),
            "#" => Some(self.positional.len().to_string()),
            "RANDOM" if !self.env.contains_key("RANDOM") => Some(self.next_random().to_string()),
            "@" | "*" => Some(self.positional.join(" ")),
            _ if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) => {
                let index: usize = name.parse().unwrap_or(usize::MAX);
                if index == 0 {
                    Some("sandbox".to_string())
                } else {
                    self.positional.get(index - 1).cloned()
                }
            }
            _ => self.env.get(name).cloned(),
        }
    }

    /// The generator state behind `$RANDOM`. A run that records it can be replayed
    /// with the same `$RANDOM` values (IMAGE.md §4.5).
    pub fn random_state(&self) -> u64 {
        self.random.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Sets the generator state behind `$RANDOM`. A zero state would repeat 0 forever,
    /// so it is replaced with a nonzero one.
    pub fn set_random_state(&self, state: u64) {
        self.random
            .store(state.max(1), std::sync::atomic::Ordering::Relaxed);
    }

    /// The next value of `$RANDOM`, 0 to 32767. An xorshift generator: it is
    /// predictable, so it is only for scripts that need variety, not secrets.
    fn next_random(&self) -> u32 {
        let mut x = self.random.load(std::sync::atomic::Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random.store(x, std::sync::atomic::Ordering::Relaxed);
        ((x >> 16) % 32768) as u32
    }

    /// `sh -c <script> [name [args...]]`. The script runs in a subshell, as `( ... )`
    /// does: its variables and directory changes do not reach this shell. Its step and
    /// output budgets are this run's, and `exit` ends only the `sh`.
    pub(crate) async fn sh_c(&mut self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (script, rest) = match args {
            [flag, script, rest @ ..] if flag == "-c" => (script, rest),
            _ => return failure_with("sh: only -c <script> is supported".to_string(), 2),
        };
        let parsed = match parser::parse(script) {
            Ok(parsed) => parsed,
            Err(e) => return failure_with(format!("sh: {}", e.message), 2),
        };
        let mut child = self.fork();
        child.positional = rest.iter().skip(1).cloned().collect();
        child.pending_stdin = Some(stdin);
        let out = child.run_script(&parsed).await;
        self.absorb(&child);
        out
    }

    /// Evaluates the text of `$((...))`.
    fn arithmetic(&self, expr: &str) -> Result<i64, String> {
        let env = &self.env;
        let positional = &self.positional;
        arith::evaluate(expr, &|name: &str| {
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
                let index: usize = name.parse().unwrap_or(usize::MAX);
                match index {
                    0 => "sandbox".to_string(),
                    n => positional.get(n - 1).cloned().unwrap_or_default(),
                }
            } else {
                env.get(name).cloned().unwrap_or_default()
            }
        })
    }

    async fn glob(&self, pattern: &[Ch]) -> Vec<String> {
        let absolute = pattern.first().is_some_and(|c| c.c == '/');
        let segments: Vec<&[Ch]> = pattern
            .split(|c| c.c == '/')
            .filter(|segment| !segment.is_empty())
            .collect();

        let mut current = vec![if absolute {
            "/".to_string()
        } else {
            String::new()
        }];
        for segment in segments {
            let mut next = Vec::new();
            for base in &current {
                next.extend(self.glob_step(segment, base).await);
            }
            current = next;
        }
        current.sort();
        current.retain(|p| !p.is_empty());
        current
    }

    /// The paths that extend `base` by one segment of a pattern. A segment with glob
    /// characters matches the entries of the directory, and a literal segment is
    /// kept when it exists.
    async fn glob_step(&self, segment: &[Ch], base: &str) -> Vec<String> {
        if !has_glob(segment) {
            let candidate = join_path(base, &text(segment));
            let exists = self
                .fs
                .stat(Path::new(&self.resolve(&candidate)))
                .await
                .is_ok();
            return if exists { vec![candidate] } else { Vec::new() };
        }
        let dir = self.resolve(if base.is_empty() { "." } else { base });
        let Ok(entries) = self.fs.readdir(Path::new(&dir)).await else {
            return Vec::new();
        };
        // A leading dot must be matched by a dot in the pattern, as in POSIX sh.
        let explicit_dot = segment.first().is_some_and(|c| c.c == '.');
        entries
            .into_iter()
            .filter(|entry| explicit_dot || !entry.name.starts_with('.'))
            .filter_map(|entry| {
                let name: Vec<char> = entry.name.chars().collect();
                glob_match(segment, &name).then(|| join_path(base, &entry.name))
            })
            .collect()
    }

    /// Absolute, normalized form of `path` relative to the current directory.
    /// `..` is resolved here, lexically and never above the root. Symlinks
    /// are followed by the VFS.
    pub(crate) fn resolve(&self, path: &str) -> String {
        let base = if path.starts_with('/') {
            ""
        } else {
            self.cwd.as_str()
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in base.split('/').chain(path.split('/')) {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        if parts.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", parts.join("/"))
        }
    }

    pub(crate) async fn read_file(&self, path: &str) -> Result<Vec<u8>, sandbox_vfs::VfsError> {
        let resolved = self.resolve(path);
        let stat = self.fs.stat(Path::new(&resolved)).await?;
        if stat.is_dir {
            return Err(sandbox_vfs::VfsError::InvalidPath(
                "Is a directory".to_string(),
            ));
        }
        let handle = self
            .fs
            .open(Path::new(&resolved), OpenFlags::default())
            .await?;
        let len = usize::try_from(stat.size).unwrap_or(usize::MAX);
        let data = self.fs.read(&handle, 0, len).await;
        self.fs.close(&handle);
        Ok(data?.to_vec())
    }

    pub(crate) async fn write_file(
        &self,
        path: &str,
        data: &[u8],
        append: bool,
    ) -> Result<(), sandbox_vfs::VfsError> {
        let resolved = self.resolve(path);
        let flags = OpenFlags {
            read: false,
            write: true,
            append,
            create: true,
            truncate: !append,
            exclusive: false,
        };
        let handle = self.fs.open(Path::new(&resolved), flags).await?;
        let written = self.fs.write(&handle, 0, data).await;
        self.fs.close(&handle);
        written.map(|_| ())
    }

    pub(crate) fn fs(&self) -> Arc<Vfs> {
        self.fs.clone()
    }

    /// The working directory as a WASI preopen, so tools see relative paths.
    pub(crate) fn mount(&self) -> sandbox_wasm::Mount {
        sandbox_wasm::Mount {
            vfs: self.fs.clone(),
            root: self.cwd.clone(),
        }
    }
}

fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// A failed command (exit 1) with a one-line message on stderr.
pub(crate) fn failure(message: String) -> ExecutionResult {
    failure_with(message, 1)
}

/// A failed command with a given exit code and a one-line message on stderr.
/// A seed for `$RANDOM`, from the clock, so each shell differs. Never zero.
fn random_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos | 1
}

pub(crate) fn failure_with(message: String, exit_code: i32) -> ExecutionResult {
    ExecutionResult {
        exit_code,
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
    }
}

/// POSIX-style text for a VFS error, for the shell user.
pub(crate) fn describe(error: &sandbox_vfs::VfsError) -> String {
    use sandbox_vfs::VfsError;
    match error {
        VfsError::NotFound(_) => "No such file or directory".to_string(),
        VfsError::AlreadyExists(_) => "File exists".to_string(),
        VfsError::InvalidPath(message) | VfsError::PermissionDenied(message) => message.clone(),
        VfsError::QuotaExceeded(message) => format!("quota exceeded: {message}"),
        other => other.to_string(),
    }
}
