//! Runs the real `wasi_tool` fixture (built for wasm32-wasip1) against a VFS.
//! Rebuild the fixture with the recipe in `fixtures/wasi_tool/Cargo.toml`.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits, WasiOutcome};
use std::path::Path;
use std::sync::Arc;

const TOOL: &[u8] = include_bytes!("../fixtures/wasi_tool.wasm");
const JQ_TOOL: &[u8] =
    include_bytes!("../fixtures/tools/target/wasm32-wasip1/release/jq_tool.wasm");
const GIT_TOOL: &[u8] =
    include_bytes!("../fixtures/tools/target/wasm32-wasip1/release/git_tool.wasm");

fn vfs() -> Arc<Vfs> {
    Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())))
}

async fn put(vfs: &Vfs, path: &str, data: &[u8]) {
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: true,
        ..OpenFlags::default()
    };
    let h = vfs.open(Path::new(path), flags).await.unwrap();
    vfs.write(&h, 0, data).await.unwrap();
    vfs.close(&h);
}

async fn get(vfs: &Vfs, path: &str) -> Vec<u8> {
    let st = vfs.stat(Path::new(path)).await.unwrap();
    let h = vfs
        .open(Path::new(path), OpenFlags::default())
        .await
        .unwrap();
    let data = vfs.read(&h, 0, st.size as usize).await.unwrap().to_vec();
    vfs.close(&h);
    data
}

async fn tool(vfs: &Arc<Vfs>, root: &str, args: &[&str], stdin: &[u8]) -> WasiOutcome {
    let mut argv = vec!["wasi_tool".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_module_with_mount(
        TOOL.to_vec(),
        argv,
        stdin.to_vec(),
        WasiLimits::default(),
        Some(Mount {
            vfs: vfs.clone(),
            root: root.to_string(),
        }),
    )
    .await
    .unwrap()
}

async fn jq_tool(vfs: &Arc<Vfs>, root: &str, args: &[&str], stdin: &[u8]) -> WasiOutcome {
    let mut argv = vec!["jq_tool".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_module_with_mount(
        JQ_TOOL.to_vec(),
        argv,
        stdin.to_vec(),
        WasiLimits::default(),
        Some(Mount {
            vfs: vfs.clone(),
            root: root.to_string(),
        }),
    )
    .await
    .unwrap()
}

async fn git_tool(vfs: &Arc<Vfs>, root: &str, args: &[&str], stdin: &[u8]) -> WasiOutcome {
    let mut argv = vec!["git_tool".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_module_with_mount(
        GIT_TOOL.to_vec(),
        argv,
        stdin.to_vec(),
        WasiLimits::default(),
        Some(Mount {
            vfs: vfs.clone(),
            root: root.to_string(),
        }),
    )
    .await
    .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn cat_reads_a_file_from_the_vfs() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    put(&vfs, "/work/hello.txt", b"from the vfs\n").await;
    let out = tool(&vfs, "/work", &["cat", "hello.txt"], b"").await;
    assert_eq!(
        (out.exit_code, text(&out.stdout).as_str()),
        (0, "from the vfs\n"),
        "{}",
        text(&out.stderr)
    );
}

#[tokio::test]
async fn write_and_append_are_visible_in_the_vfs() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    let out = tool(&vfs, "/work", &["write", "out.txt", "one"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let out = tool(&vfs, "/work", &["append", "out.txt", " two"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert_eq!(get(&vfs, "/work/out.txt").await, b"one two");
}

#[tokio::test]
async fn ls_and_stat_see_the_vfs_tree() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    put(&vfs, "/work/b", b"").await;
    put(&vfs, "/work/a", b"xyz").await;
    vfs.mkdir(Path::new("/work/d"), 0o755).await.unwrap();

    let out = tool(&vfs, "/work", &["ls", "."], b"").await;
    assert_eq!(text(&out.stdout), "a\nb\nd\n");

    let out = tool(&vfs, "/work", &["stat", "a"], b"").await;
    assert_eq!(text(&out.stdout), "file 3\n");
    let out = tool(&vfs, "/work", &["stat", "d"], b"").await;
    assert_eq!(text(&out.stdout), "dir 0\n");
}

#[tokio::test]
async fn directory_and_file_management_reach_the_vfs() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    assert_eq!(
        tool(&vfs, "/work", &["mkdir", "sub"], b"").await.exit_code,
        0
    );
    assert!(vfs.stat(Path::new("/work/sub")).await.unwrap().is_dir);

    put(&vfs, "/work/sub/f", b"data").await;
    assert_eq!(
        tool(&vfs, "/work", &["mv", "sub/f", "g"], b"")
            .await
            .exit_code,
        0
    );
    assert_eq!(get(&vfs, "/work/g").await, b"data");

    assert_eq!(tool(&vfs, "/work", &["rm", "g"], b"").await.exit_code, 0);
    assert!(vfs.stat(Path::new("/work/g")).await.is_err());

    assert_eq!(
        tool(&vfs, "/work", &["rmdir", "sub"], b"").await.exit_code,
        0
    );
    assert!(vfs.stat(Path::new("/work/sub")).await.is_err());
}

#[tokio::test]
async fn removing_a_non_empty_directory_fails() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    vfs.mkdir(Path::new("/work/full"), 0o755).await.unwrap();
    put(&vfs, "/work/full/x", b"").await;
    let out = tool(&vfs, "/work", &["rmdir", "full"], b"").await;
    assert_eq!(out.exit_code, 1);
    assert!(vfs.stat(Path::new("/work/full/x")).await.is_ok());
}

#[tokio::test]
async fn stdin_stdout_and_stderr_are_in_memory() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    let out = tool(&vfs, "/work", &["stdin"], b"piped in").await;
    assert_eq!(text(&out.stdout), "piped in");
    let out = tool(&vfs, "/work", &["stderr"], b"").await;
    assert_eq!(text(&out.stderr), "to stderr\n");
}

#[tokio::test]
async fn guest_exit_codes_and_argv_come_through() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    assert_eq!(tool(&vfs, "/work", &["exit", "7"], b"").await.exit_code, 7);
    let out = tool(&vfs, "/work", &["args", "x", "y"], b"").await;
    assert_eq!(text(&out.stdout), "args,x,y\n");
}

#[tokio::test]
async fn the_guest_cannot_leave_its_mount() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    put(&vfs, "/secret", b"outside").await;

    for escape in ["/secret", "../secret", "sub/../../secret"] {
        let out = tool(&vfs, "/work", &["cat", escape], b"").await;
        assert_eq!(out.exit_code, 1, "{escape} must fail");
        assert!(out.stdout.is_empty(), "{escape} must not leak data");
    }
}

#[tokio::test]
async fn without_a_mount_the_guest_has_no_files() {
    let outcome = run_module_with_mount(
        TOOL.to_vec(),
        vec!["wasi_tool".into(), "cat".into(), "x".into()],
        Vec::new(),
        WasiLimits::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(outcome.exit_code, 1);
}

#[tokio::test]
async fn host_handles_are_released_after_a_run() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    put(&vfs, "/work/f", b"x").await;
    // `cat` opens and closes its file. A leak would show as a handle left in the VFS.
    tool(&vfs, "/work", &["cat", "f"], b"").await;
    tool(&vfs, "/work", &["cat", "f"], b"").await;
    assert_eq!(vfs.open_handle_count(), 0);
}

#[tokio::test]
async fn jq_tool_processes_json() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    // Test identity filter via stdin
    let out = jq_tool(
        &vfs,
        "/work",
        &["."],
        b"{\"name\": \"test\", \"value\": 42, \"tags\": [\"a\", \"b\"]}",
    )
    .await;
    eprintln!("jq_tool identity stdout: {}", text(&out.stdout));
    eprintln!("jq_tool identity stderr: {}", text(&out.stderr));
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("test"),
        "stdout: {}",
        text(&out.stdout)
    );
    assert!(
        text(&out.stdout).contains("42"),
        "stdout: {}",
        text(&out.stdout)
    );

    // Test field access via stdin
    let out = jq_tool(
        &vfs,
        "/work",
        &[".name"],
        b"{\"name\": \"test\", \"value\": 42}",
    )
    .await;
    eprintln!("jq_tool .name stdout: {}", text(&out.stdout));
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "\"test\"");

    // Test array access via stdin
    let out = jq_tool(&vfs, "/work", &[".tags"], b"{\"tags\": [\"a\", \"b\"]}").await;
    eprintln!("jq_tool .tags stdout: {}", text(&out.stdout));
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("a"));
    assert!(text(&out.stdout).contains("b"));
}

#[tokio::test]
async fn jq_tool_handles_stdin() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    // Test with stdin
    let out = jq_tool(&vfs, "/work", &[".value"], b"{\"value\": 123}").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "123");
}

#[tokio::test]
async fn git_tool_basic_commands() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    // Test init
    let out = git_tool(&vfs, "/work", &["init"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Initialized empty Git repository"));

    // Test status
    let out = git_tool(&vfs, "/work", &["status"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("On branch main"));

    // Test log
    let out = git_tool(&vfs, "/work", &["log", "--oneline"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Initial commit"));
}

#[tokio::test]
async fn git_tool_commit_flow() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    // Initialize
    git_tool(&vfs, "/work", &["init"], b"").await;

    // Create a file
    put(&vfs, "/work/hello.txt", b"hello world").await;

    // Add file
    let out = git_tool(&vfs, "/work", &["add", "hello.txt"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));

    // Commit
    let out = git_tool(&vfs, "/work", &["commit", "-m", "Add hello"], b"").await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Created commit"));

    // Log
    let out = git_tool(&vfs, "/work", &["log"], b"").await;
    eprintln!("git_tool log stdout: {}", text(&out.stdout));
    eprintln!("git_tool log stderr: {}", text(&out.stderr));
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Initial commit"));
}

#[tokio::test]
async fn git_tool_unknown_command() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();

    let out = git_tool(&vfs, "/work", &["unknown"], b"").await;
    assert_eq!(out.exit_code, 1, "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("unknown command"));
}
