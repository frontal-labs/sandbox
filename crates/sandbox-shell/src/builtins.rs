//! Builtin commands, implemented against the VFS.

use crate::interpreter::{describe, failure, ExecutionResult, Flow, Input, Shell};
use crate::{expand, printf};
use sandbox_vfs::{SessionFs, VfsError};
use std::collections::BTreeSet;
use std::path::Path;

/// Every builtin name. `type` reports these, so keep it in step with `builtin`.
const BUILTINS: &[&str] = &[
    "true", "false", "echo", "printf", "read", "pwd", "cd", "cat", "ls", "mkdir", "rm", "mv", "cp",
    "touch", "head", "tail", "wc", "grep", "node", "wasm", "tools", "sandbox", "sh", ":", "test",
    "[", "break", "continue", "return", "exit", "export", "unset", "type",
];

/// Whether `name` is a builtin.
pub(crate) fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

fn ok(stdout: Vec<u8>) -> ExecutionResult {
    ExecutionResult {
        exit_code: 0,
        stdout,
        stderr: Vec::new(),
    }
}

/// Splits `-abc` style flags from operands. Only letters in `allowed` are accepted.
fn parse_flags<'a>(
    command: &str,
    args: &'a [String],
    allowed: &str,
) -> Result<(BTreeSet<char>, Vec<&'a str>), String> {
    let mut flags = BTreeSet::new();
    let mut operands = Vec::new();
    for arg in args {
        if arg.len() > 1 && arg.starts_with('-') {
            for c in arg[1..].chars() {
                if !allowed.contains(c) {
                    return Err(format!("{command}: invalid option -- '{c}'"));
                }
                flags.insert(c);
            }
        } else {
            operands.push(arg.as_str());
        }
    }
    Ok((flags, operands))
}

/// Like `failure`, with an exit status of the caller's choosing.
fn failure_status(message: String, exit_code: i32) -> ExecutionResult {
    ExecutionResult {
        exit_code,
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
    }
}

/// `read` without `-r`: a backslash makes the next character literal.
fn unescape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits a line for `read NAME...`. Each name but the last takes one word, and
/// the last takes the rest of the line, with leading and trailing blanks removed.
fn split_for_names(names: &[&str], line: &str) -> Vec<(String, String)> {
    let blanks = |c: char| expand::IFS.contains(&c);
    let mut rest = line.trim_matches(blanks);
    let mut values = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if i + 1 == names.len() {
            values.push((
                (*name).to_string(),
                rest.trim_end_matches(blanks).to_string(),
            ));
        } else {
            let (word, after) = match rest.find(blanks) {
                Some(end) => (&rest[..end], rest[end..].trim_start_matches(blanks)),
                None => (rest, ""),
            };
            values.push(((*name).to_string(), word.to_string()));
            rest = after;
        }
    }
    values
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Shell {
    /// Runs a builtin, or returns `None` if `argv[0]` is not one.
    pub(crate) async fn builtin(
        &mut self,
        argv: &[String],
        stdin: Vec<u8>,
    ) -> Option<ExecutionResult> {
        let name = argv.first()?.as_str();
        let args = &argv[1..];
        let out = match name {
            "true" => ok(Vec::new()),
            "false" => ExecutionResult {
                exit_code: 1,
                ..ExecutionResult::default()
            },
            "echo" => self.echo(args),
            "printf" => self.printf(args),
            "read" => self.read(args, stdin),
            "grep" => self.grep(args, stdin).await,
            "exit" => self.exit_(args),
            "type" => self.type_(args),
            "pwd" => ok(format!("{}\n", self.cwd()).into_bytes()),
            "cd" => self.cd(args).await,
            "cat" => self.cat(args, stdin).await,
            "ls" => self.ls(args).await,
            "mkdir" => self.mkdir(args).await,
            "rm" => self.rm(args).await,
            "mv" => self.mv(args).await,
            "cp" => self.cp(args).await,
            "touch" => self.touch(args).await,
            "head" => self.head(args, stdin).await,
            "tail" => self.tail(args, stdin).await,
            "wc" => self.wc(args, stdin).await,
            "node" => self.node(args, stdin).await,
            "wasm" => self.wasm(args, stdin).await,
            "tools" => self.tools().await,
            "sandbox" => self.sandbox(args).await,
            "sh" => self.sh_c(args, stdin).await,
            ":" => ok(Vec::new()),
            "test" => self.test(args).await,
            "[" => match args.split_last() {
                Some((last, rest)) if last == "]" => self.test(rest).await,
                _ => failure("[: missing ']'".to_string()),
            },
            "break" => self.loop_control(args, Flow::Break),
            "continue" => self.loop_control(args, Flow::Continue),
            "return" => self.return_(args),
            "export" => self.export(args),
            "unset" => self.unset(args),
            _ => return None,
        };
        Some(out)
    }

    /// `echo [-neE] [WORD]...`. `-n` drops the newline, `-e` expands escapes,
    /// and `-E` turns that back off. Other arguments are printed as they are.
    fn echo(&self, args: &[String]) -> ExecutionResult {
        let mut newline = true;
        let mut escapes = false;
        let mut index = 0;
        while let Some(flags) = args.get(index).and_then(|arg| arg.strip_prefix('-')) {
            if flags.is_empty() || !flags.chars().all(|c| matches!(c, 'n' | 'e' | 'E')) {
                break;
            }
            for flag in flags.chars() {
                match flag {
                    'n' => newline = false,
                    'e' => escapes = true,
                    _ => escapes = false,
                }
            }
            index += 1;
        }
        let mut text = args[index..].join(" ");
        if escapes {
            let (expanded, stop) = printf::echo_escapes(&text);
            text = expanded;
            if stop {
                newline = false;
            }
        }
        if newline {
            text.push('\n');
        }
        ok(text.into_bytes())
    }

    /// `printf FORMAT [ARGUMENT]...`
    fn printf(&self, args: &[String]) -> ExecutionResult {
        let Some((format_text, rest)) = args.split_first() else {
            return failure("printf: usage: printf FORMAT [ARGUMENT]...".to_string());
        };
        match printf::format(format_text, rest) {
            Ok(printf::Formatted {
                output,
                error: None,
            }) => ok(output),
            Ok(printf::Formatted {
                output,
                error: Some(message),
            }) => ExecutionResult {
                exit_code: 1,
                stdout: output,
                stderr: format!("{message}\n").into_bytes(),
            },
            Err(message) => failure(message),
        }
    }

    /// `read [-r] NAME...`: reads one line from the input the shell is reading,
    /// or from this command's stdin. Status 1 at the end of the input. The last
    /// name gets the rest of the line.
    fn read(&mut self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, names) = match parse_flags("read", args, "r") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        if let Some(bad) = names.iter().find(|name| !expand::is_name(name)) {
            return failure(format!("read: `{bad}': not a valid identifier"));
        }
        let (line, complete) = match self.input.as_mut() {
            Some(input) => input.next_line(),
            None => Input::new(stdin).next_line(),
        }
        .unwrap_or_default();
        let mut line = String::from_utf8_lossy(&line).into_owned();
        if !flags.contains(&'r') {
            line = unescape_line(&line);
        }
        if names.is_empty() {
            self.set_env("REPLY".to_string(), line);
        } else {
            for (name, value) in split_for_names(&names, &line) {
                self.set_env(name, value);
            }
        }
        ExecutionResult {
            exit_code: i32::from(!complete),
            ..ExecutionResult::default()
        }
    }

    /// `exit [N]`. Ends the shell, or the subshell it runs in.
    fn exit_(&mut self, args: &[String]) -> ExecutionResult {
        let code = match args.first() {
            None => self.status(),
            Some(arg) => match arg.trim().parse::<i32>() {
                Ok(code) => code & 0xff,
                Err(_) => {
                    self.flow = Some(Flow::Exit(2));
                    return ExecutionResult {
                        exit_code: 2,
                        stdout: Vec::new(),
                        stderr: format!("exit: {arg}: numeric argument required\n").into_bytes(),
                    };
                }
            },
        };
        self.flow = Some(Flow::Exit(code));
        ExecutionResult {
            exit_code: code,
            ..ExecutionResult::default()
        }
    }

    /// `type NAME...`: says what each name is. Status 1 if one is not found.
    fn type_(&self, args: &[String]) -> ExecutionResult {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut status = 0;
        for name in args {
            if self.has_function(name) {
                stdout.push_str(&format!("{name} is a function\n"));
            } else if BUILTINS.contains(&name.as_str()) {
                stdout.push_str(&format!("{name} is a shell builtin\n"));
            } else {
                stderr.push_str(&format!("type: {name}: not found\n"));
                status = 1;
            }
        }
        ExecutionResult {
            exit_code: status,
            stdout: stdout.into_bytes(),
            stderr: stderr.into_bytes(),
        }
    }

    /// `grep [-FiqvnlcE] PATTERN [FILE]...`. PATTERN is a regular expression
    /// (the syntax of the `regex` crate, which is close to POSIX extended). `-F`
    /// matches it as text. Status 0 if a line is selected, 1 if none is, and 2
    /// on error, which takes precedence unless `-q` found a match.
    async fn grep(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("grep", args, "FiqvnlcE") {
            Ok(parsed) => parsed,
            Err(message) => return failure_status(message, 2),
        };
        let Some((pattern, files)) = operands.split_first() else {
            return failure_status(
                "grep: usage: grep [-FiqvnlcE] PATTERN [FILE]...".to_string(),
                2,
            );
        };
        let source = if flags.contains(&'F') {
            regex::escape(pattern)
        } else {
            pattern.to_string()
        };
        let regex = match regex::RegexBuilder::new(&source)
            .case_insensitive(flags.contains(&'i'))
            .build()
        {
            Ok(regex) => regex,
            Err(e) => return failure_status(format!("grep: {pattern}: {e}"), 2),
        };

        let mut inputs: Vec<(Option<String>, Vec<u8>)> = Vec::new();
        let mut stderr = String::new();
        let mut error = false;
        if files.is_empty() {
            inputs.push((None, stdin));
        }
        for file in files {
            match self.read_file(file).await {
                Ok(data) => inputs.push((Some((*file).to_string()), data)),
                Err(e) => {
                    error = true;
                    stderr.push_str(&format!("grep: {file}: {}\n", describe(&e)));
                }
            }
        }

        let label = inputs.len() > 1;
        let mut stdout = String::new();
        let mut any = false;
        for (name, data) in &inputs {
            let text = String::from_utf8_lossy(data);
            let mut count = 0usize;
            for (number, line) in text.lines().enumerate() {
                if regex.is_match(line) == flags.contains(&'v') {
                    continue;
                }
                count += 1;
                if flags.contains(&'q') || flags.contains(&'l') || flags.contains(&'c') {
                    continue;
                }
                if label {
                    stdout.push_str(name.as_deref().unwrap_or("(standard input)"));
                    stdout.push(':');
                }
                if flags.contains(&'n') {
                    stdout.push_str(&format!("{}:", number + 1));
                }
                stdout.push_str(line);
                stdout.push('\n');
            }
            any |= count > 0;
            if flags.contains(&'c') {
                if label {
                    stdout.push_str(name.as_deref().unwrap_or("(standard input)"));
                    stdout.push(':');
                }
                stdout.push_str(&format!("{count}\n"));
            }
            if flags.contains(&'l') && count > 0 {
                stdout.push_str(name.as_deref().unwrap_or("(standard input)"));
                stdout.push('\n');
            }
        }

        let status = if error && !(flags.contains(&'q') && any) {
            2
        } else {
            i32::from(!any)
        };
        ExecutionResult {
            exit_code: status,
            stdout: if flags.contains(&'q') {
                Vec::new()
            } else {
                stdout.into_bytes()
            },
            stderr: stderr.into_bytes(),
        }
    }

    async fn cd(&mut self, args: &[String]) -> ExecutionResult {
        if args.len() > 1 {
            return failure("cd: too many arguments".to_string());
        }
        let target = args.first().map(String::as_str).unwrap_or("/");
        match self.chdir(target).await {
            Ok(()) => ok(Vec::new()),
            Err(e) => failure(format!("cd: {}", e.message)),
        }
    }

    /// `sandbox info` prints the session's env.json (IMAGE.md §4.5). Checkpoint, undo
    /// and fork need the server, so the shell refuses them and names the host tool.
    async fn sandbox(&self, args: &[String]) -> ExecutionResult {
        match args.first().map(String::as_str) {
            Some("info") => match self.read_file("/etc/agent/env.json").await {
                Ok(data) => ok(data),
                Err(e) => ExecutionResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: format!("sandbox: {}\n", describe(&e)).into_bytes(),
                },
            },
            Some(other @ ("checkpoint" | "undo" | "fork")) => ExecutionResult {
                exit_code: 2,
                stdout: Vec::new(),
                stderr: format!(
                    "sandbox {other}: needs the server; run `frontal-sandbox sandbox {other}` from the host\n"
                )
                .into_bytes(),
            },
            _ => ExecutionResult {
                exit_code: 2,
                stdout: Vec::new(),
                stderr: b"usage: sandbox info | checkpoint | undo | fork\n".to_vec(),
            },
        }
    }

    async fn cat(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        if args.is_empty() {
            return ok(stdin);
        }
        let mut out = Vec::new();
        let mut errors = Vec::new();
        let mut stdin = Some(stdin);
        for arg in args {
            if arg == "-" {
                out.extend(stdin.take().unwrap_or_default());
                continue;
            }
            match self.read_file(arg).await {
                Ok(data) => out.extend(data),
                Err(e) => errors.push(format!("cat: {arg}: {}", describe(&e))),
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out,
            stderr: errors
                .iter()
                .flat_map(|l| format!("{l}\n").into_bytes())
                .collect(),
        }
    }

    async fn ls(&self, args: &[String]) -> ExecutionResult {
        let (_, operands) = match parse_flags("ls", args, "") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        let operands = if operands.is_empty() {
            vec!["."]
        } else {
            operands
        };
        let multiple = operands.len() > 1;

        let mut out = String::new();
        let mut errors = Vec::new();
        for (i, operand) in operands.iter().enumerate() {
            let resolved = self.resolve(operand);
            let stat = match self.fs().stat(Path::new(&resolved)).await {
                Ok(stat) => stat,
                Err(e) => {
                    errors.push(format!("ls: {operand}: {}", describe(&e)));
                    continue;
                }
            };
            if !stat.is_dir {
                out.push_str(&format!("{operand}\n"));
                continue;
            }
            let entries = match self.fs().readdir(Path::new(&resolved)).await {
                Ok(entries) => entries,
                Err(e) => {
                    errors.push(format!("ls: {operand}: {}", describe(&e)));
                    continue;
                }
            };
            if multiple {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&format!("{operand}:\n"));
            }
            for entry in entries {
                out.push_str(&entry.name);
                out.push('\n');
            }
        }
        ExecutionResult {
            exit_code: i32::from(!errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: errors
                .iter()
                .flat_map(|l| format!("{l}\n").into_bytes())
                .collect(),
        }
    }

    async fn mkdir(&self, args: &[String]) -> ExecutionResult {
        let (flags, operands) = match parse_flags("mkdir", args, "p") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        if operands.is_empty() {
            return failure("mkdir: missing operand".to_string());
        }
        for operand in operands {
            let resolved = self.resolve(operand);
            let result = if flags.contains(&'p') {
                self.mkdir_parents(&resolved).await
            } else {
                self.fs().mkdir(Path::new(&resolved), 0o755).await
            };
            if let Err(e) = result {
                return failure(format!("mkdir: {operand}: {}", describe(&e)));
            }
        }
        ok(Vec::new())
    }

    async fn mkdir_parents(&self, resolved: &str) -> Result<(), VfsError> {
        let mut prefix = String::new();
        for part in resolved.split('/').filter(|p| !p.is_empty()) {
            prefix.push('/');
            prefix.push_str(part);
            match self.fs().mkdir(Path::new(&prefix), 0o755).await {
                Ok(()) | Err(VfsError::AlreadyExists(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    async fn rm(&self, args: &[String]) -> ExecutionResult {
        let (flags, operands) = match parse_flags("rm", args, "rf") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        if operands.is_empty() {
            return failure("rm: missing operand".to_string());
        }
        let recursive = flags.contains(&'r');
        let force = flags.contains(&'f');
        for operand in operands {
            let resolved = self.resolve(operand);
            if !recursive {
                match self.fs().stat(Path::new(&resolved)).await {
                    Ok(stat) if stat.is_dir => {
                        return failure(format!("rm: {operand}: is a directory"));
                    }
                    Err(VfsError::NotFound(_)) if force => continue,
                    _ => {}
                }
            }
            match self.fs().rm(Path::new(&resolved), recursive).await {
                Ok(()) => {}
                Err(VfsError::NotFound(_)) if force => {}
                Err(e) => return failure(format!("rm: {operand}: {}", describe(&e))),
            }
        }
        ok(Vec::new())
    }

    /// If `dst` names an existing directory, the source goes inside it.
    async fn destination(&self, src: &str, dst: &str) -> String {
        let resolved = self.resolve(dst);
        match self.fs().stat(Path::new(&resolved)).await {
            Ok(stat) if stat.is_dir => format!("{resolved}/{}", basename(src)),
            _ => resolved,
        }
    }

    async fn mv(&self, args: &[String]) -> ExecutionResult {
        let (_, operands) = match parse_flags("mv", args, "") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        let [src, dst] = operands[..] else {
            return failure("mv: expected a source and a destination".to_string());
        };
        let from = self.resolve(src);
        let to = self.destination(src, dst).await;
        // Like mv, replace a file that is already at the destination.
        if let Ok(stat) = self.fs().stat(Path::new(&to)).await {
            if !stat.is_dir {
                if let Err(e) = self.fs().rm(Path::new(&to), false).await {
                    return failure(format!("mv: {dst}: {}", describe(&e)));
                }
            }
        }
        match self.fs().rename(Path::new(&from), Path::new(&to)).await {
            Ok(()) => ok(Vec::new()),
            Err(e) => failure(format!("mv: {src}: {}", describe(&e))),
        }
    }

    async fn cp(&self, args: &[String]) -> ExecutionResult {
        let (_, operands) = match parse_flags("cp", args, "") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        let [src, dst] = operands[..] else {
            return failure("cp: expected a source and a destination".to_string());
        };
        match self.fs().stat(Path::new(&self.resolve(src))).await {
            Ok(stat) if stat.is_dir => {
                return failure(format!("cp: {src}: recursive copy is not supported"));
            }
            Ok(_) => {}
            Err(e) => return failure(format!("cp: {src}: {}", describe(&e))),
        }
        let data = match self.read_file(src).await {
            Ok(data) => data,
            Err(e) => return failure(format!("cp: {src}: {}", describe(&e))),
        };
        let to = self.destination(src, dst).await;
        match self.write_resolved(&to, &data).await {
            Ok(()) => ok(Vec::new()),
            Err(e) => failure(format!("cp: {dst}: {}", describe(&e))),
        }
    }

    async fn touch(&self, args: &[String]) -> ExecutionResult {
        let (_, operands) = match parse_flags("touch", args, "") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        if operands.is_empty() {
            return failure("touch: missing file operand".to_string());
        }
        for operand in operands {
            let flags = sandbox_vfs::OpenFlags {
                read: false,
                write: true,
                create: true,
                ..sandbox_vfs::OpenFlags::default()
            };
            match self
                .fs()
                .open(Path::new(&self.resolve(operand)), flags)
                .await
            {
                Ok(handle) => self.fs().close(&handle),
                Err(e) => return failure(format!("touch: {operand}: {}", describe(&e))),
            }
        }
        ok(Vec::new())
    }

    /// `tail [-n N] [FILE]...`: the last N lines (default 10).
    async fn tail(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut count = 10usize;
        let mut rest = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-n" {
                match iter.next().and_then(|v| v.parse().ok()) {
                    Some(n) => count = n,
                    None => return failure("tail: option -n needs a number".to_string()),
                }
            } else {
                rest.push(arg.clone());
            }
        }
        let inputs = self.read_inputs("tail", &rest, stdin).await;
        let mut out = Vec::new();
        for data in inputs.data {
            let lines: Vec<&[u8]> = data.split_inclusive(|&b| b == b'\n').collect();
            let first = lines.len().saturating_sub(count);
            out.extend(lines[first..].iter().flat_map(|line| line.iter().copied()));
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    async fn head(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let mut count = 10usize;
        let mut rest = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-n" {
                match iter.next().and_then(|v| v.parse().ok()) {
                    Some(n) => count = n,
                    None => return failure("head: option -n needs a number".to_string()),
                }
            } else {
                rest.push(arg.clone());
            }
        }
        let inputs = self.read_inputs("head", &rest, stdin).await;
        let mut out = Vec::new();
        for data in inputs.data {
            out.extend(data.split_inclusive(|&b| b == b'\n').take(count).flatten());
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out,
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    async fn wc(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let (flags, operands) = match parse_flags("wc", args, "lwc") {
            Ok(parsed) => parsed,
            Err(message) => return failure(message),
        };
        // With no flags, all three counts are shown.
        let all = flags.is_empty();
        let inputs = self
            .read_inputs(
                "wc",
                &operands.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                stdin,
            )
            .await;
        let mut out = String::new();
        for (i, data) in inputs.data.iter().enumerate() {
            let text = String::from_utf8_lossy(data);
            let mut line = String::new();
            if all || flags.contains(&'l') {
                line.push_str(&format!("{:>8}", text.matches('\n').count()));
            }
            if all || flags.contains(&'w') {
                line.push_str(&format!("{:>8}", text.split_whitespace().count()));
            }
            if all || flags.contains(&'c') {
                line.push_str(&format!("{:>8}", data.len()));
            }
            if let Some(name) = operands.get(i) {
                line.push_str(&format!(" {name}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        ExecutionResult {
            exit_code: i32::from(!inputs.errors.is_empty()),
            stdout: out.into_bytes(),
            stderr: inputs.errors.join("\n").into_bytes(),
        }
    }

    /// Reads each file operand, or stdin when there are none.
    async fn read_inputs(&self, command: &str, operands: &[String], stdin: Vec<u8>) -> Inputs {
        if operands.is_empty() {
            return Inputs {
                data: vec![stdin],
                errors: Vec::new(),
            };
        }
        let mut inputs = Inputs {
            data: Vec::new(),
            errors: Vec::new(),
        };
        for operand in operands {
            match self.read_file(operand).await {
                Ok(data) => inputs.data.push(data),
                Err(e) => inputs
                    .errors
                    .push(format!("{command}: {operand}: {}", describe(&e))),
            }
        }
        inputs
    }

    async fn write_resolved(&self, resolved: &str, data: &[u8]) -> Result<(), VfsError> {
        let flags = sandbox_vfs::OpenFlags {
            read: false,
            write: true,
            create: true,
            truncate: true,
            ..sandbox_vfs::OpenFlags::default()
        };
        let handle = self.fs().open(Path::new(resolved), flags).await?;
        let written = self.fs().write(&handle, 0, data).await;
        self.fs().close(&handle);
        written.map(|_| ())
    }
}

impl Shell {
    /// `break [N]` and `continue [N]`: leave or skip N enclosing loops.
    fn loop_control(&mut self, args: &[String], make: fn(u32) -> Flow) -> ExecutionResult {
        let n = match args.first() {
            None => 1,
            Some(arg) => match arg.parse::<u32>() {
                Ok(n) if n >= 1 => n,
                _ => return failure(format!("loop count must be a positive number: {arg}")),
            },
        };
        self.flow = Some(make(n));
        ok(Vec::new())
    }

    /// `return [N]`: end the current function with status N (default: last status).
    fn return_(&mut self, args: &[String]) -> ExecutionResult {
        let code = match args.first() {
            None => self.status(),
            Some(arg) => match arg.parse::<i32>() {
                Ok(code) => code,
                Err(_) => return failure(format!("return: {arg}: numeric argument required")),
            },
        };
        self.flow = Some(Flow::Return(code));
        ExecutionResult {
            exit_code: code,
            ..ExecutionResult::default()
        }
    }

    /// `export NAME[=value]...`. Shell variables are the environment, so this
    /// sets the value. `export NAME` alone only validates the name.
    fn export(&mut self, args: &[String]) -> ExecutionResult {
        for arg in args {
            let (name, value) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (arg.as_str(), None),
            };
            if !crate::expand::is_name(name) {
                return failure(format!("export: {name}: not a valid name"));
            }
            if let Some(value) = value {
                self.set_env(name.to_string(), value.to_string());
            }
        }
        ok(Vec::new())
    }

    /// `unset NAME...`
    fn unset(&mut self, args: &[String]) -> ExecutionResult {
        for name in args {
            self.unset_env(name);
        }
        ok(Vec::new())
    }

    /// `test EXPR` and `[ EXPR ]`. Supports `! -z -n -f -d -e -s = == != -eq -ne -lt -le -gt -ge`.
    async fn test(&self, args: &[String]) -> ExecutionResult {
        match self.eval_test(args).await {
            Ok(true) => ok(Vec::new()),
            Ok(false) => ExecutionResult {
                exit_code: 1,
                ..ExecutionResult::default()
            },
            Err(message) => ExecutionResult {
                exit_code: 2,
                stdout: Vec::new(),
                stderr: format!("test: {message}\n").into_bytes(),
            },
        }
    }

    async fn eval_test(&self, args: &[String]) -> Result<bool, String> {
        let arg = |i: usize| args[i].as_str();
        match args.len() {
            0 => Ok(false),
            1 => Ok(!arg(0).is_empty()),
            2 if arg(0) == "!" => Ok(!Box::pin(self.eval_test(&args[1..])).await?),
            2 => self.unary_test(arg(0), arg(1)).await,
            3 => match arg(1) {
                "=" | "==" => Ok(arg(0) == arg(2)),
                "!=" => Ok(arg(0) != arg(2)),
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                    let a: i64 = arg(0)
                        .parse()
                        .map_err(|_| format!("{}: integer expression expected", arg(0)))?;
                    let b: i64 = arg(2)
                        .parse()
                        .map_err(|_| format!("{}: integer expression expected", arg(2)))?;
                    Ok(match arg(1) {
                        "-eq" => a == b,
                        "-ne" => a != b,
                        "-lt" => a < b,
                        "-le" => a <= b,
                        "-gt" => a > b,
                        _ => a >= b,
                    })
                }
                _ if arg(0) == "!" => Ok(!self.unary_test(arg(1), arg(2)).await?),
                op => Err(format!("unknown operator {op}")),
            },
            _ => Err("too many arguments".to_string()),
        }
    }

    async fn unary_test(&self, op: &str, operand: &str) -> Result<bool, String> {
        match op {
            "-z" => Ok(operand.is_empty()),
            "-n" => Ok(!operand.is_empty()),
            "-f" | "-d" | "-e" | "-s" => {
                let stat = self.stat_path(operand).await;
                Ok(match (op, stat) {
                    (_, None) => false,
                    ("-f", Some(s)) => s.is_file,
                    ("-d", Some(s)) => s.is_dir,
                    ("-s", Some(s)) => s.size > 0,
                    (_, Some(_)) => true,
                })
            }
            _ => Err(format!("unknown operator {op}")),
        }
    }

    /// Lists the tools in the registry: `name version path`.
    async fn tools(&self) -> ExecutionResult {
        match self.registry().await {
            Ok(registry) => {
                let mut out = String::new();
                for tool in &registry.tools {
                    let version = tool.version.as_deref().unwrap_or("-");
                    out.push_str(&format!("{} {} {}\n", tool.name, version, tool.path));
                }
                ok(out.into_bytes())
            }
            Err(e) => failure(format!("tools: {}", e.message)),
        }
    }

    /// Runs JavaScript in a fresh V8 isolate. This is not Node.js: only
    /// `console.log` is provided, and there is no filesystem or module access.
    async fn node(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let code = match args {
            [] => String::from_utf8_lossy(&stdin).into_owned(),
            [flag, code] if flag == "-e" => code.clone(),
            [file] if !file.starts_with('-') => {
                let source = match self.read_file(file).await {
                    Ok(data) => String::from_utf8_lossy(&data).into_owned(),
                    Err(e) => return failure(format!("node: {file}: {}", describe(&e))),
                };
                // A `.ts` file runs as the JavaScript its types erase to.
                if file.ends_with(".ts") {
                    match sandbox_runtime::strip_types(&source, file) {
                        Ok(js) => js,
                        Err(e) => return failure(format!("node: {}", e.message)),
                    }
                } else {
                    source
                }
            }
            _ => return failure("node: usage: node [-e CODE | FILE]".to_string()),
        };
        match sandbox_runtime::execute_script_contained(
            &code,
            sandbox_runtime::ScriptLimits::default(),
            self.tool_stdout(),
        )
        .await
        {
            Ok(out) => ExecutionResult {
                exit_code: out.exit_code,
                stdout: out.stdout,
                stderr: out.stderr,
            },
            Err(e) => failure(format!("node: {}", e.message)),
        }
    }
}

impl Shell {
    /// Runs a WASI command module from the VFS: `wasm FILE [ARGS...]`. The
    /// module gets argv `[FILE, ARGS...]`, stdin, and no filesystem access.
    async fn wasm(&self, args: &[String], stdin: Vec<u8>) -> ExecutionResult {
        let Some(file) = args.first() else {
            return failure("wasm: usage: wasm FILE [ARGS...]".to_string());
        };
        let module = match self.read_file(file).await {
            Ok(bytes) => bytes,
            Err(e) => return failure(format!("wasm: {file}: {}", describe(&e))),
        };
        match sandbox_wasm::run_module_with_mount_streamed(
            module,
            args.to_vec(),
            stdin,
            sandbox_wasm::WasiLimits::default(),
            Some(self.mount()),
            self.tool_stdout(),
        )
        .await
        {
            Ok(out) => ExecutionResult {
                exit_code: out.exit_code,
                stdout: out.stdout,
                stderr: out.stderr,
            },
            Err(e) => failure(format!("wasm: {}", e.message)),
        }
    }
}

struct Inputs {
    data: Vec<Vec<u8>>,
    errors: Vec<String>,
}
