//! Runs parsed scripts against a VFS.
//!
//! Command lookup order: function, builtin, then a WASI tool (the registry at
//! `TOOLS_MANIFEST`, then `PATH`, default `/bin`), then exit 127. The shell
//! never runs host binaries. Every command counts against a step budget, so
//! a loop cannot hang the host.

use crate::ast::{
    Ch, Command, Connector, ForClause, IfClause, Piece, Pipeline, RedirectKind, Script,
    SimpleCommand, WhileClause, Word,
};
use crate::expand::{expand_vars, glob_match, has_glob, split_assignment, text, Scope};
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
    pub max_output_bytes: usize,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            cwd: "/".to_string(),
            env: HashMap::new(),
            max_steps: 1_000_000,
            max_output_bytes: 1024 * 1024,
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
    Abort,
}

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
    output_used: usize,
    max_output_bytes: usize,
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
            steps: 0,
            max_steps: config.max_steps,
            output_used: 0,
            max_output_bytes: config.max_output_bytes,
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
        let script: Script = parser::parse(script)?;
        self.steps = 0;
        self.output_used = 0;
        let result = self.run_script(&script).await;
        self.flow = None;
        Ok(result)
    }

    /// Runs `argv` as one command, with `stdin` as its input. Arguments are
    /// passed through unexpanded.
    pub async fn run_argv(
        &mut self,
        argv: &[String],
        stdin: &[u8],
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
        let out = self.run_simple(&command).await;
        self.pending_stdin = None;
        self.flow = None;
        self.status = out.exit_code;
        Ok(out)
    }

    /// A copy of the shell state, for subshells and command substitutions.
    /// Changes made to the copy do not reach this shell.
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
            steps: self.steps,
            max_steps: self.max_steps,
            output_used: self.output_used,
            max_output_bytes: self.max_output_bytes,
        }
    }

    fn run_script<'a>(&'a mut self, script: &'a Script) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            let mut result = ExecutionResult::default();
            let mut previous = Connector::Seq;
            for item in &script.items {
                if self.flow.is_some() {
                    break;
                }
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
            // A compound command as a stage passes its input to its first command.
            let mut input = self.pending_stdin.take().unwrap_or_default();
            let mut stderr = Vec::new();
            let mut exit_code = 0;
            for stage in &pipeline.stages {
                self.pending_stdin = Some(std::mem::take(&mut input));
                let out = self.run_command(stage).await;
                self.pending_stdin = None;
                stderr.extend(out.stderr);
                exit_code = out.exit_code;
                input = out.stdout;
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
            match command {
                Command::Simple(simple) => self.run_simple(simple).await,
                Command::If(clause) => self.run_if(clause).await,
                Command::While(clause) => self.run_while(clause).await,
                Command::For(clause) => self.run_for(clause).await,
                Command::Group(body) => self.run_script(body).await,
                Command::Subshell(body) => {
                    let mut child = self.fork();
                    let out = child.run_script(body).await;
                    self.steps = child.steps;
                    self.output_used = child.output_used;
                    out
                }
                Command::Function(def) => {
                    self.functions.insert(def.name.clone(), (*def.body).clone());
                    ExecutionResult::default()
                }
            }
        })
    }

    async fn run_if(&mut self, clause: &IfClause) -> ExecutionResult {
        let mut out = ExecutionResult::default();
        for (condition, body) in &clause.branches {
            merge(&mut out, self.run_script(condition).await);
            if self.flow.is_some() {
                break;
            }
            if self.status == 0 {
                merge(&mut out, self.run_script(body).await);
                out.exit_code = self.status;
                return out;
            }
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
        loop {
            merge(&mut out, self.run_script(&clause.condition).await);
            if self.flow.is_some() {
                break;
            }
            let succeeded = self.status == 0;
            if succeeded == clause.until {
                break;
            }
            merge(&mut out, self.run_script(&clause.body).await);
            if self.loop_should_stop() {
                break;
            }
        }
        out.exit_code = self.status;
        out
    }

    async fn run_for(&mut self, clause: &ForClause) -> ExecutionResult {
        let mut out = ExecutionResult::default();
        let mut values = Vec::new();
        for word in &clause.words {
            let (fields, expansion_stderr) = self.expand_word(word).await;
            out.stderr.extend(expansion_stderr);
            values.extend(fields);
        }
        for value in values {
            self.env.insert(clause.variable.clone(), value);
            merge(&mut out, self.run_script(&clause.body).await);
            if self.loop_should_stop() {
                break;
            }
        }
        out.exit_code = self.status;
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
            Some(Flow::Return(_) | Flow::Abort) => true,
        }
    }

    async fn run_simple(&mut self, command: &SimpleCommand) -> ExecutionResult {
        let mut stderr = Vec::new();
        let mut stdin = self.pending_stdin.take().unwrap_or_default();

        // Leading `NAME=value` words are assignments. The rest is the command.
        let mut assignments = Vec::new();
        let mut index = 0;
        while let Some(word) = command.words.get(index) {
            match split_assignment(word) {
                Some((name, value)) => {
                    let (chars, subst_err) = self.expand_chars(&value).await;
                    stderr.extend(subst_err);
                    assignments.push((name, text(&chars)));
                    index += 1;
                }
                None => break,
            }
        }

        let mut argv = Vec::new();
        for word in &command.words[index..] {
            let (fields, subst_err) = self.expand_word(word).await;
            stderr.extend(subst_err);
            argv.extend(fields);
        }

        let mut targets: Vec<(RedirectKind, String)> = Vec::new();
        for redirect in &command.redirects {
            let (chars, subst_err) = self.expand_chars(&redirect.target).await;
            stderr.extend(subst_err);
            targets.push((redirect.kind, text(&chars)));
        }

        if argv.is_empty() {
            // Assignments alone persist in the shell. Redirections with no
            // command only create or truncate their target files.
            for (name, value) in assignments {
                self.env.insert(name, value);
            }
            for (kind, target) in &targets {
                if *kind == RedirectKind::Output {
                    if let Err(e) = self.write_file(target, &[], false).await {
                        stderr.extend(format!("{target}: {}\n", describe(&e)).into_bytes());
                    }
                }
            }
            return ExecutionResult {
                exit_code: i32::from(!stderr.is_empty()),
                stdout: Vec::new(),
                stderr,
            };
        }

        // Input redirections load stdin before the command runs. The last one wins.
        for (kind, target) in &targets {
            if *kind == RedirectKind::Input {
                match self.read_file(target).await {
                    Ok(data) => stdin = data,
                    Err(e) => return failure(format!("{target}: {}", describe(&e))),
                }
            }
        }

        // Prefix assignments apply to this command only.
        let saved: Vec<(String, Option<String>)> = assignments
            .iter()
            .map(|(name, _)| (name.clone(), self.env.get(name).cloned()))
            .collect();
        for (name, value) in assignments {
            self.env.insert(name, value);
        }

        let mut out = self.run_argv_inner(&argv, stdin).await;

        for (name, previous) in saved.into_iter().rev() {
            match previous {
                Some(value) => self.env.insert(name, value),
                None => self.env.remove(&name),
            };
        }

        let outputs: Vec<&(RedirectKind, String)> = targets
            .iter()
            .filter(|(kind, _)| *kind != RedirectKind::Input)
            .collect();
        let mut exit_code = out.exit_code;
        if !outputs.is_empty() {
            let last = outputs.len() - 1;
            for (i, (kind, target)) in outputs.iter().enumerate() {
                let data = if i == last {
                    std::mem::take(&mut out.stdout)
                } else {
                    Vec::new()
                };
                let append = *kind == RedirectKind::Append;
                if let Err(e) = self.write_file(target, &data, append).await {
                    out.stderr
                        .extend(format!("{target}: {}\n", describe(&e)).into_bytes());
                    exit_code = 1;
                }
            }
        }
        stderr.extend(out.stderr);
        let mut result = ExecutionResult {
            exit_code,
            stdout: out.stdout,
            stderr,
        };
        self.charge_output(&mut result);
        result
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
            return self.call_function(body, &argv[1..], stdin).await;
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
    async fn expand_word(&mut self, word: &Word) -> (Vec<String>, Vec<u8>) {
        let (chars, stderr) = self.expand_chars(word).await;
        let quoted = word.0.iter().any(|piece| match piece {
            Piece::Ch(ch) => ch.mode != crate::ast::Mode::Bare,
            Piece::Subst(_) => false,
        });
        if has_glob(&chars) {
            let matches = self.glob(&chars).await;
            if !matches.is_empty() {
                return (matches, stderr);
            }
        }
        if chars.is_empty() && !quoted {
            return (Vec::new(), stderr);
        }
        (vec![text(&chars)], stderr)
    }

    /// Expands a word's variables and substitutions into characters. No globbing.
    async fn expand_chars(&mut self, word: &Word) -> (Vec<Ch>, Vec<u8>) {
        let mut stderr = Vec::new();
        let mut out: Vec<Ch> = Vec::new();
        let mut run: Vec<Ch> = Vec::new();
        for piece in &word.0 {
            match piece {
                Piece::Ch(ch) => run.push(*ch),
                Piece::Subst(script) => {
                    out.extend(self.expand_run(&run));
                    run.clear();
                    let (output, sub_stderr) = self.substitute(script).await;
                    stderr.extend(sub_stderr);
                    out.extend(output);
                }
            }
        }
        out.extend(self.expand_run(&run));
        (out, stderr)
    }

    fn expand_run(&self, run: &[Ch]) -> Vec<Ch> {
        let scope = Scope {
            env: &self.env,
            positional: &self.positional,
            status: self.status,
        };
        expand_vars(run, &scope)
    }

    /// Runs a command substitution in a copy of the shell and returns its
    /// output as literal characters, minus trailing newlines.
    async fn substitute(&mut self, script: &Script) -> (Vec<Ch>, Vec<u8>) {
        let mut child = self.fork();
        let out = child.run_script(script).await;
        self.steps = child.steps;
        self.output_used = child.output_used;
        let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
        while output.ends_with('\n') {
            output.pop();
        }
        let chars = output
            .chars()
            .map(|c| Ch {
                c,
                mode: crate::ast::Mode::Literal,
            })
            .collect();
        (chars, out.stderr)
    }

    /// Expands a pattern against the VFS. Returns sorted matches, or an
    /// empty list if nothing matches (the caller then keeps the literal word).
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
                if has_glob(segment) {
                    let dir = self.resolve(if base.is_empty() { "." } else { base });
                    let Ok(entries) = self.fs.readdir(Path::new(&dir)).await else {
                        continue;
                    };
                    let explicit_dot = segment.first().is_some_and(|c| c.c == '.');
                    for entry in entries {
                        if entry.name.starts_with('.') && !explicit_dot {
                            continue;
                        }
                        let name: Vec<char> = entry.name.chars().collect();
                        if glob_match(segment, &name) {
                            next.push(join_path(base, &entry.name));
                        }
                    }
                } else {
                    let candidate = join_path(base, &text(segment));
                    if self
                        .fs
                        .stat(Path::new(&self.resolve(&candidate)))
                        .await
                        .is_ok()
                    {
                        next.push(candidate);
                    }
                }
            }
            current = next;
        }
        current.sort();
        current.retain(|p| !p.is_empty());
        current
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

    pub(crate) fn fs(&self) -> &Vfs {
        &self.fs
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
