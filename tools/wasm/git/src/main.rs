//! `git_tool`: a WASI build of a git subset for the sandbox (PLAN.md section 4.6).
//!
//! It writes real git repositories: loose objects, the v2 index, and refs under
//! `.git`. Host git reads them. The porcelain output follows git's own, so a
//! conformance test can compare it with host git. Differences are listed in
//! `README.md`. Among them: no pack files, no remotes, no `.gitignore`, and the
//! default identity `Sandbox <sandbox@frontal.local>` when none is configured.

mod index;
mod object;

use index::{Index, IndexEntry, REGULAR_FILE};
use object::{encode_commit, encode_tree, hash_object, hex, parse_commit, parse_tree, sort_tree_entries, Commit, Kind, ObjectStore, Oid, TreeEntry};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A failure. `code` 128 is git's `fatal:` class, and prints with that prefix. An empty
/// message prints nothing, for commands that have already written their output.
#[derive(Debug)]
pub struct Fatal {
    pub code: i32,
    pub message: String,
}

impl Fatal {
    pub fn new(message: impl Into<String>) -> Self {
        Fatal {
            code: 128,
            message: message.into(),
        }
    }

    fn silent(code: i32) -> Self {
        Fatal {
            code,
            message: String::new(),
        }
    }
}

type Result<T> = std::result::Result<T, Fatal>;

const BRANCH: &str = "main";
const DEFAULT_NAME: &str = "Sandbox";
const DEFAULT_EMAIL: &str = "sandbox@frontal.local";

/// Options given before the subcommand, such as `-c user.name=...`.
#[derive(Default)]
struct Globals {
    config: BTreeMap<String, String>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => {}
        Err(error) => {
            if error.code == 128 {
                eprintln!("fatal: {}", error.message);
            } else if !error.message.is_empty() {
                eprintln!("{}", error.message);
            }
            std::process::exit(error.code);
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let mut globals = Globals::default();
    let mut rest = args;
    while let Some((first, tail)) = rest.split_first() {
        if first == "-c" {
            let (pair, after) = tail
                .split_first()
                .ok_or_else(|| Fatal::new("-c needs a key=value argument"))?;
            add_config(&mut globals, pair)?;
            rest = after;
        } else if let Some(pair) = first.strip_prefix("-c") {
            add_config(&mut globals, pair)?;
            rest = tail;
        } else {
            break;
        }
    }
    let Some((command, operands)) = rest.split_first() else {
        return Err(Fatal::silent(1));
    };
    match command.as_str() {
        "init" => cmd_init(operands),
        "add" => cmd_add(operands),
        "commit" => cmd_commit(&globals, operands),
        "status" => cmd_status(operands),
        "log" => cmd_log(operands),
        "write-tree" => cmd_write_tree(),
        "ls-files" => cmd_ls_files(operands),
        "hash-object" => cmd_hash_object(operands),
        "clone" | "fetch" | "pull" | "push" | "remote" => Err(Fatal::new(format!(
            "{command} needs network access, which this sandbox does not provide"
        ))),
        other => Err(Fatal {
            code: 1,
            message: format!("git: '{other}' is not a git command. See 'git --help'."),
        }),
    }
}

fn add_config(globals: &mut Globals, pair: &str) -> Result<()> {
    let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| Fatal::new(format!("bad config entry '{pair}'")))?;
    globals.config.insert(key.to_string(), value.to_string());
    Ok(())
}

/// The repository in the current directory, and its object store.
struct Repo {
    git_dir: PathBuf,
    objects: ObjectStore,
}

impl Repo {
    fn open() -> Result<Repo> {
        let git_dir = PathBuf::from(".git");
        if !git_dir.is_dir() {
            return Err(Fatal::new(
                "not a git repository (or any parent up to mount point /)",
            ));
        }
        Ok(Repo {
            objects: ObjectStore::new(&git_dir),
            git_dir,
        })
    }

    fn head_branch(&self) -> Result<String> {
        let head = read_text(&self.git_dir.join("HEAD"))?;
        head.trim()
            .strip_prefix("ref: refs/heads/")
            .map(str::to_string)
            .ok_or_else(|| Fatal::new("HEAD is detached, which this tool does not support"))
    }

    fn branch_path(&self, branch: &str) -> PathBuf {
        self.git_dir.join("refs").join("heads").join(branch)
    }

    /// The commit the current branch points to, or `None` before the first commit.
    fn head_commit(&self) -> Result<Option<Oid>> {
        let path = self.branch_path(&self.head_branch()?);
        if !path.exists() {
            return Ok(None);
        }
        let text = read_text(&path)?;
        object::parse_hex(text.trim())
            .map(Some)
            .ok_or_else(|| Fatal::new("the branch ref is corrupt"))
    }

    fn load_index(&self) -> Result<Index> {
        let path = self.git_dir.join("index");
        if !path.exists() {
            return Ok(Index::default());
        }
        let bytes = fs::read(&path).map_err(|e| Fatal::new(format!("cannot read index: {e}")))?;
        Index::parse(&bytes)
    }

    fn save_index(&self, index: &Index) -> Result<()> {
        write_file(&self.git_dir.join("index"), &index.encode())
    }

    /// The files of the head commit's tree, by path.
    fn head_files(&self) -> Result<BTreeMap<String, (u32, Oid)>> {
        let mut files = BTreeMap::new();
        if let Some(commit) = self.head_commit()? {
            let commit = self.read_commit(&commit)?;
            self.flatten(&commit.tree, "", &mut files)?;
        }
        Ok(files)
    }

    fn flatten(&self, tree: &Oid, prefix: &str, out: &mut BTreeMap<String, (u32, Oid)>) -> Result<()> {
        let (kind, body) = self.objects.read(tree)?;
        if kind != Kind::Tree {
            return Err(Fatal::new("expected a tree object"));
        }
        for entry in parse_tree(&body)? {
            let name = utf8(&entry.name)?;
            let path = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.is_tree() {
                self.flatten(&entry.oid, &path, out)?;
            } else {
                out.insert(path, (entry.mode, entry.oid));
            }
        }
        Ok(())
    }

    fn read_commit(&self, oid: &Oid) -> Result<Commit> {
        let (kind, body) = self.objects.read(oid)?;
        if kind != Kind::Commit {
            return Err(Fatal::new("expected a commit object"));
        }
        parse_commit(&body)
    }

    /// Writes the index as nested tree objects, and returns the root tree.
    fn write_index_tree(&self, index: &Index) -> Result<Oid> {
        let mut root = Dir::default();
        for (path, entry) in &index.entries {
            root.insert(path, entry.mode, entry.oid);
        }
        self.write_dir(&root)
    }

    fn write_dir(&self, dir: &Dir) -> Result<Oid> {
        let mut entries = Vec::new();
        for (name, node) in &dir.children {
            match node {
                Node::File { mode, oid } => entries.push(TreeEntry {
                    mode: *mode,
                    name: name.as_bytes().to_vec(),
                    oid: *oid,
                }),
                Node::Dir(sub) => entries.push(TreeEntry {
                    mode: 0o40000,
                    name: name.as_bytes().to_vec(),
                    oid: self.write_dir(sub)?,
                }),
            }
        }
        sort_tree_entries(&mut entries);
        self.objects.write(Kind::Tree, &encode_tree(&entries))
    }
}

#[derive(Default)]
struct Dir {
    children: BTreeMap<String, Node>,
}

enum Node {
    File { mode: u32, oid: Oid },
    Dir(Dir),
}

impl Dir {
    fn insert(&mut self, path: &str, mode: u32, oid: Oid) {
        match path.split_once('/') {
            None => {
                self.children
                    .insert(path.to_string(), Node::File { mode, oid });
            }
            Some((head, rest)) => {
                let node = self
                    .children
                    .entry(head.to_string())
                    .or_insert_with(|| Node::Dir(Dir::default()));
                if let Node::Dir(sub) = node {
                    sub.insert(rest, mode, oid);
                }
            }
        }
    }
}

fn cmd_init(operands: &[String]) -> Result<()> {
    let quiet = operands.iter().any(|arg| arg == "-q" || arg == "--quiet");
    let base = match operands.iter().find(|arg| !arg.starts_with('-')) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from("."),
    };
    let git_dir = base.join(".git");
    let existed = git_dir.is_dir();
    for dir in ["objects", "refs/heads", "refs/tags"] {
        fs::create_dir_all(git_dir.join(dir))
            .map_err(|e| Fatal::new(format!("cannot create {dir}: {e}")))?;
    }
    if !existed {
        write_file(&git_dir.join("HEAD"), format!("ref: refs/heads/{BRANCH}\n").as_bytes())?;
        write_file(
            &git_dir.join("config"),
            b"[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n",
        )?;
    }
    let verb = if existed {
        "Reinitialized existing"
    } else {
        "Initialized empty"
    };
    let shown = if base == Path::new(".") {
        ".git/".to_string()
    } else {
        format!("{}/.git/", base.display())
    };
    if !quiet {
        println!("{verb} Git repository in {shown}");
    }
    Ok(())
}

fn cmd_add(operands: &[String]) -> Result<()> {
    if operands.is_empty() {
        eprintln!("Nothing specified, nothing added.\nhint: Maybe you wanted to say 'git add .'?");
        return Ok(());
    }
    let repo = Repo::open()?;
    let worktree = list_worktree()?;
    let mut index = repo.load_index()?;
    for spec in operands {
        let spec = normalize(spec);
        let matches_worktree = worktree.iter().any(|path| matches(&spec, path));
        let matches_index = index.entries.keys().any(|path| matches(&spec, path));
        if !matches_worktree && !matches_index {
            return Err(Fatal::new(format!(
                "pathspec '{spec}' did not match any files"
            )));
        }
    }
    for spec in operands {
        let spec = normalize(spec);
        for path in worktree.iter().filter(|path| matches(&spec, path)) {
            let bytes = fs::read(path).map_err(|e| Fatal::new(format!("cannot read {path}: {e}")))?;
            let oid = repo.objects.write(Kind::Blob, &bytes)?;
            index.entries.insert(
                path.clone(),
                IndexEntry {
                    mode: REGULAR_FILE,
                    oid,
                    size: bytes.len() as u32,
                },
            );
        }
        // A tracked file that is gone from the worktree is staged as deleted.
        let gone: Vec<String> = index
            .entries
            .keys()
            .filter(|path| matches(&spec, path) && !worktree.contains(path))
            .cloned()
            .collect();
        for path in gone {
            index.entries.remove(&path);
        }
    }
    repo.save_index(&index)
}

fn cmd_commit(globals: &Globals, operands: &[String]) -> Result<()> {
    let mut messages = Vec::new();
    let mut author = None;
    let mut args = operands.iter();
    while let Some(arg) = args.next() {
        if arg == "-m" {
            messages.push(
                args.next()
                    .cloned()
                    .ok_or_else(|| Fatal::new("option '-m' requires a value"))?,
            );
        } else if let Some(value) = arg.strip_prefix("-m") {
            messages.push(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--author=") {
            author = Some(parse_identity(value)?);
        } else {
            return Err(Fatal::new(format!("unsupported option '{arg}'")));
        }
    }
    if messages.is_empty() {
        return Err(Fatal::new("no commit message given (use -m)"));
    }
    let repo = Repo::open()?;
    let index = repo.load_index()?;
    let tree = repo.write_index_tree(&index)?;
    let head = repo.head_commit()?;
    let nothing_staged = match head {
        Some(parent) => repo.read_commit(&parent)?.tree == tree,
        None => index.entries.is_empty(),
    };
    if nothing_staged {
        // git prints the status here, so the message comes with the branch line.
        let changes = changes(&repo)?;
        print!("{}", render_status(&repo.head_branch()?, head.is_some(), &changes));
        return Err(Fatal::silent(1));
    }

    let (name, email) = match author {
        Some(identity) => identity,
        None => (
            globals
                .config
                .get("user.name")
                .cloned()
                .unwrap_or_else(|| DEFAULT_NAME.to_string()),
            globals
                .config
                .get("user.email")
                .cloned()
                .unwrap_or_else(|| DEFAULT_EMAIL.to_string()),
        ),
    };
    let stamp = format!("{name} <{email}> {} +0000", now_seconds());
    let message = format!("{}\n", messages.join("\n\n").trim_end());
    let commit = Commit {
        tree,
        parents: head.into_iter().collect(),
        author: stamp.clone(),
        committer: stamp,
        message: message.clone(),
    };
    let oid = repo.objects.write(Kind::Commit, &encode_commit(&commit))?;
    write_file(&repo.branch_path(&repo.head_branch()?), format!("{}\n", hex(&oid)).as_bytes())?;
    let subject = message.lines().next().unwrap_or_default();
    let root = if head.is_none() { " (root-commit)" } else { "" };
    println!("[{BRANCH}{root} {}] {subject}", index::short(&oid));
    Ok(())
}

fn cmd_status(operands: &[String]) -> Result<()> {
    let mut porcelain = false;
    let mut branch_header = false;
    for arg in operands {
        match arg.as_str() {
            "--porcelain" | "--porcelain=v1" => porcelain = true,
            "-b" | "--branch" => branch_header = true,
            _ => return Err(Fatal::new(format!("unsupported status option '{arg}'"))),
        }
    }
    let repo = Repo::open()?;
    let branch = repo.head_branch()?;
    let has_commits = repo.head_commit()?.is_some();
    let changes = changes(&repo)?;
    let mut out = String::new();
    if porcelain {
        if branch_header {
            let header = if has_commits {
                format!("## {branch}")
            } else {
                format!("## No commits yet on {branch}")
            };
            out.push_str(&header);
            out.push('\n');
        }
        for line in changes.porcelain_lines() {
            out.push_str(&line);
            out.push('\n');
        }
    } else {
        out.push_str(&render_status(&branch, has_commits, &changes));
    }
    print!("{out}");
    io::stdout().flush().map_err(|e| Fatal::new(e.to_string()))
}

/// The long status format. Sections are separated by blank lines, and a clean tree
/// with commits gets its one line right under the branch line.
fn render_status(branch: &str, has_commits: bool, changes: &Changes) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !has_commits {
        sections.push("No commits yet".to_string());
    }
    if !changes.staged.is_empty() {
        let unstage = if has_commits {
            "git restore --staged"
        } else {
            "git rm --cached"
        };
        let mut lines = vec![
            "Changes to be committed:".to_string(),
            format!("  (use \"{unstage} <file>...\" to unstage)"),
        ];
        lines.extend(changes.staged.iter().map(|(code, path)| {
            format!("\t{:<12}{path}", staged_label(*code))
        }));
        sections.push(lines.join("\n"));
    }
    if !changes.unstaged.is_empty() {
        // git says `add/rm` when a tracked file was deleted, because `add` stages deletions too.
        let verb = if changes.unstaged.iter().any(|(code, _)| *code == 'D') {
            "add/rm"
        } else {
            "add"
        };
        let mut lines = vec![
            "Changes not staged for commit:".to_string(),
            format!("  (use \"git {verb} <file>...\" to update what will be committed)"),
            "  (use \"git restore <file>...\" to discard changes in working directory)".to_string(),
        ];
        lines.extend(changes.unstaged.iter().map(|(code, path)| {
            format!("\t{:<12}{path}", unstaged_label(*code))
        }));
        sections.push(lines.join("\n"));
    }
    if !changes.untracked.is_empty() {
        let mut lines = vec![
            "Untracked files:".to_string(),
            "  (use \"git add <file>...\" to include in what will be committed)".to_string(),
        ];
        lines.extend(changes.untracked.iter().map(|path| format!("\t{path}")));
        sections.push(lines.join("\n"));
    }
    if let Some(text) = summary(changes, has_commits) {
        sections.push(text);
    }
    // A blank line goes before every section, except the first one after the branch
    // line when there are commits. A clean tree is the one line under the branch.
    let mut out = format!("On branch {branch}\n");
    for (index, section) in sections.iter().enumerate() {
        if index > 0 || !has_commits {
            out.push('\n');
        }
        out.push_str(section);
        out.push('\n');
    }
    out
}

fn summary(changes: &Changes, has_commits: bool) -> Option<String> {
    if !changes.staged.is_empty() {
        None
    } else if !changes.unstaged.is_empty() {
        Some("no changes added to commit (use \"git add\" and/or \"git commit -a\")".to_string())
    } else if !changes.untracked.is_empty() {
        Some("nothing added to commit but untracked files present (use \"git add\" to track)".to_string())
    } else if has_commits {
        Some("nothing to commit, working tree clean".to_string())
    } else {
        Some("nothing to commit (create/copy files and use \"git add\" to track)".to_string())
    }
}

fn staged_label(code: char) -> &'static str {
    match code {
        'A' => "new file:",
        'D' => "deleted:",
        _ => "modified:",
    }
}

fn unstaged_label(code: char) -> &'static str {
    match code {
        'D' => "deleted:",
        _ => "modified:",
    }
}

/// What differs between HEAD, the index, and the worktree.
struct Changes {
    /// Index against HEAD, with `A`, `M`, or `D` per path.
    staged: Vec<(char, String)>,
    /// Worktree against the index, with `M` or `D` per path.
    unstaged: Vec<(char, String)>,
    /// Untracked paths. A directory with nothing tracked inside is shown once, with a `/`.
    untracked: Vec<String>,
}

impl Changes {
    fn porcelain_lines(&self) -> Vec<String> {
        let mut by_path: BTreeMap<String, (char, char)> = BTreeMap::new();
        for (code, path) in &self.staged {
            by_path.entry(path.clone()).or_insert((' ', ' ')).0 = *code;
        }
        for (code, path) in &self.unstaged {
            by_path.entry(path.clone()).or_insert((' ', ' ')).1 = *code;
        }
        let mut lines: Vec<(String, String)> = by_path
            .into_iter()
            .map(|(path, (x, y))| (path.clone(), format!("{x}{y} {path}")))
            .collect();
        // git lists tracked changes first, each group sorted by path.
        let mut untracked: Vec<(String, String)> = self
            .untracked
            .iter()
            .map(|path| (path.clone(), format!("?? {path}")))
            .collect();
        lines.sort_by(|a, b| a.0.cmp(&b.0));
        untracked.sort_by(|a, b| a.0.cmp(&b.0));
        lines
            .into_iter()
            .chain(untracked)
            .map(|(_, line)| line)
            .collect()
    }
}

fn changes(repo: &Repo) -> Result<Changes> {
    let head = repo.head_files()?;
    let index = repo.load_index()?;
    let worktree = list_worktree()?;
    let mut work: BTreeMap<String, Oid> = BTreeMap::new();
    for path in &worktree {
        let bytes = fs::read(path).map_err(|e| Fatal::new(format!("cannot read {path}: {e}")))?;
        work.insert(path.clone(), hash_object(Kind::Blob, &bytes));
    }

    let paths: BTreeSet<&String> = head.keys().chain(index.entries.keys()).collect();
    let mut staged = Vec::new();
    for path in paths {
        match (head.get(path), index.entries.get(path)) {
            (None, Some(_)) => staged.push(('A', path.clone())),
            (Some(_), None) => staged.push(('D', path.clone())),
            (Some((mode, oid)), Some(entry)) if *oid != entry.oid || *mode != entry.mode => {
                staged.push(('M', path.clone()));
            }
            _ => {}
        }
    }

    let mut unstaged = Vec::new();
    for (path, entry) in &index.entries {
        match work.get(path) {
            None => unstaged.push(('D', path.clone())),
            Some(oid) if *oid != entry.oid => unstaged.push(('M', path.clone())),
            Some(_) => {}
        }
    }

    let untracked_files: Vec<&String> = worktree
        .iter()
        .filter(|path| !index.entries.contains_key(*path))
        .collect();
    let mut untracked = BTreeSet::new();
    for path in untracked_files {
        untracked.insert(collapse_untracked(path, &index));
    }
    Ok(Changes {
        staged,
        unstaged,
        untracked: untracked.into_iter().collect(),
    })
}

/// The path to show for an untracked file. The highest directory that holds no
/// tracked file is shown as `dir/`, as git shows it.
fn collapse_untracked(path: &str, index: &Index) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = parts.len().saturating_sub(1);
    (1..=dirs)
        .map(|depth| parts[..depth].join("/"))
        .find(|dir| {
            let prefix = format!("{dir}/");
            !index.entries.keys().any(|tracked| tracked.starts_with(&prefix))
        })
        .map(|dir| format!("{dir}/"))
        .unwrap_or_else(|| path.to_string())
}

fn cmd_log(operands: &[String]) -> Result<()> {
    let mut oneline = false;
    let mut format: Option<String> = None;
    let mut limit = usize::MAX;
    let mut args = operands.iter();
    while let Some(arg) = args.next() {
        if arg == "--oneline" {
            oneline = true;
        } else if let Some(value) = arg.strip_prefix("--format=") {
            format = Some(value.to_string());
        } else if arg == "-n" {
            limit = args
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Fatal::new("-n needs a number"))?;
        } else if let Some(value) = arg.strip_prefix('-').and_then(|v| v.parse().ok()) {
            limit = value;
        } else {
            return Err(Fatal::new(format!("unsupported log option '{arg}'")));
        }
    }
    let repo = Repo::open()?;
    let Some(head) = repo.head_commit()? else {
        return Err(Fatal::new(format!(
            "your current branch '{BRANCH}' does not have any commits yet"
        )));
    };
    let mut out = String::new();
    let mut cursor = Some(head);
    let mut shown = 0usize;
    while let (Some(oid), true) = (cursor, shown < limit) {
        let commit = repo.read_commit(&oid)?;
        out.push_str(&format_commit(&oid, &commit, oneline, format.as_deref(), shown == 0));
        cursor = commit.parents.first().copied();
        shown += 1;
    }
    print!("{out}");
    io::stdout().flush().map_err(|e| Fatal::new(e.to_string()))
}

/// One commit in `log` output. `first` tells whether a separator goes before it.
fn format_commit(oid: &Oid, commit: &Commit, oneline: bool, format: Option<&str>, first: bool) -> String {
    let subject = commit.message.lines().next().unwrap_or_default();
    if let Some(template) = format {
        return format!("{}\n", expand(template, oid, commit));
    }
    if oneline {
        return format!("{} {subject}\n", index::short(oid));
    }
    let (identity, when) = split_stamp(&commit.author);
    let separator = if first { "" } else { "\n" };
    let body: String = commit
        .message
        .trim_end_matches('\n')
        .lines()
        .map(|line| format!("    {line}\n"))
        .collect();
    format!(
        "{separator}commit {}\nAuthor: {identity}\nDate:   {when}\n\n{body}",
        hex(oid)
    )
}

/// Splits a stamp, `Name <email> seconds offset`, into the identity and git's `Date:` text.
fn split_stamp(stamp: &str) -> (String, String) {
    let fields: Vec<&str> = stamp.rsplitn(3, ' ').collect();
    match fields.as_slice() {
        [offset, seconds, identity] => (
            (*identity).to_string(),
            git_date(seconds.parse::<i64>().unwrap_or(0), offset),
        ),
        _ => (stamp.to_string(), git_date(0, "+0000")),
    }
}

fn expand(template: &str, oid: &Oid, commit: &Commit) -> String {
    let subject = commit.message.lines().next().unwrap_or_default();
    template
        .replace("%H", &hex(oid))
        .replace("%h", &index::short(oid))
        .replace("%s", subject)
}

fn cmd_write_tree() -> Result<()> {
    let repo = Repo::open()?;
    let index = repo.load_index()?;
    let tree = repo.write_index_tree(&index)?;
    println!("{}", hex(&tree));
    Ok(())
}

fn cmd_ls_files(operands: &[String]) -> Result<()> {
    if operands != ["--stage"] {
        return Err(Fatal::new("ls-files supports only --stage"));
    }
    let repo = Repo::open()?;
    for (path, entry) in &repo.load_index()?.entries {
        println!("{:06o} {} 0\t{path}", entry.mode, hex(&entry.oid));
    }
    Ok(())
}

fn cmd_hash_object(operands: &[String]) -> Result<()> {
    let write = operands.iter().any(|arg| arg == "-w");
    let files: Vec<&String> = operands.iter().filter(|arg| *arg != "-w").collect();
    let [file] = files.as_slice() else {
        return Err(Fatal::new("hash-object takes one file"));
    };
    let bytes = fs::read(file).map_err(|e| Fatal::new(format!("cannot read {file}: {e}")))?;
    let oid = if write {
        Repo::open()?.objects.write(Kind::Blob, &bytes)?
    } else {
        hash_object(Kind::Blob, &bytes)
    };
    println!("{}", hex(&oid));
    Ok(())
}

/// Every regular file under the current directory, as sorted relative paths. `.git` is skipped.
fn list_worktree() -> Result<Vec<String>> {
    let mut files = Vec::new();
    walk(Path::new("."), "", &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| Fatal::new(format!("cannot read {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| Fatal::new(e.to_string()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Fatal::new("file name is not valid UTF-8"))?;
        if prefix.is_empty() && name == ".git" {
            continue;
        }
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = entry.metadata().map_err(|e| Fatal::new(e.to_string()))?;
        if metadata.is_dir() {
            walk(&entry.path(), &path, out)?;
        } else if metadata.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// A pathspec without `./` or a trailing `/`. `.` matches everything.
fn normalize(spec: &str) -> String {
    let trimmed = spec.trim_start_matches("./").trim_end_matches('/');
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}

fn matches(spec: &str, path: &str) -> bool {
    spec == "." || path == spec || path.starts_with(&format!("{spec}/"))
}

fn parse_identity(text: &str) -> Result<(String, String)> {
    let (name, rest) = text
        .split_once(" <")
        .ok_or_else(|| Fatal::new("--author must look like 'Name <email>'"))?;
    let email = rest
        .strip_suffix('>')
        .ok_or_else(|| Fatal::new("--author must look like 'Name <email>'"))?;
    Ok((name.to_string(), email.to_string()))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// git's `Date:` format, in the stored offset: `Tue Nov  3 21:00:00 2026 +0000`.
fn git_date(seconds: i64, offset: &str) -> String {
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let digits: i64 = offset.trim_start_matches(['+', '-']).parse().unwrap_or(0);
    let local = seconds + sign * ((digits / 100) * 3600 + (digits % 100) * 60);
    let days = local.div_euclid(86_400);
    let clock = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][(days + 4).rem_euclid(7) as usize];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{weekday} {} {day} {:02}:{:02}:{:02} {year} {offset}",
        months[(month - 1) as usize],
        clock / 3600,
        clock % 3600 / 60,
        clock % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| Fatal::new(format!("cannot read {}: {e}", path.display())))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|e| Fatal::new(format!("cannot write {}: {e}", path.display())))
}

fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| Fatal::new("path is not valid UTF-8"))
}
