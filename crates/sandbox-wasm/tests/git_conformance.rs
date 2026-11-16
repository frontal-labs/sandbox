//! The `git` WASI tool against host git 2.52. The expected values are host git's output
//! for the same files, so the object hashes, index, tree, and porcelain status must match
//! exactly. Commit hashes depend on the clock, so they are not compared here.
//! `tools/wasm/git/conformance.sh` runs wider scenarios against host git.

use sandbox_cas::MemStore;
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use sandbox_wasm::{run_module_with_mount, Mount, WasiLimits, WasiOutcome};
use std::path::Path;
use std::sync::Arc;

const GIT_TOOL: &[u8] = include_bytes!("../fixtures/git_tool.wasm");

fn vfs() -> Arc<Vfs> {
    Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())))
}

async fn put(vfs: &Vfs, path: &str, data: &[u8], append: bool) {
    let flags = OpenFlags {
        read: false,
        write: true,
        create: true,
        truncate: !append,
        ..OpenFlags::default()
    };
    let h = vfs.open(Path::new(path), flags).await.unwrap();
    let at = if append {
        vfs.stat(Path::new(path)).await.unwrap().size
    } else {
        0
    };
    vfs.write(&h, at, data).await.unwrap();
    vfs.close(&h);
}

async fn git(vfs: &Arc<Vfs>, args: &[&str]) -> WasiOutcome {
    let mut argv = vec!["git_tool".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_module_with_mount(
        GIT_TOOL.to_vec(),
        argv,
        Vec::new(),
        WasiLimits::default(),
        Some(Mount {
            vfs: vfs.clone(),
            root: "/work".to_string(),
        }),
    )
    .await
    .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn the_wasm_tool_matches_host_git_on_hashes_index_and_status() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    for dir in ["/work/a", "/work/a/b", "/work/a/b/c"] {
        vfs.mkdir(Path::new(dir), 0o755).await.unwrap();
    }
    put(&vfs, "/work/a/b/c/deep.txt", b"1\n", false).await;
    put(&vfs, "/work/a/top.txt", b"2\n", false).await;
    put(&vfs, "/work/root.txt", b"3\n", false).await;

    let out = git(&vfs, &["add", "a", "root.txt"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let out = git(
        &vfs,
        &[
            "-c",
            "user.name=Tester",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-m",
            "first",
        ],
    )
    .await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));

    let clean = git(&vfs, &["status", "--porcelain"]).await;
    assert_eq!(
        text(&clean.stdout),
        "",
        "a committed tree has no status lines"
    );

    put(&vfs, "/work/root.txt", b"more\n", true).await;
    put(&vfs, "/work/new.txt", b"n\n", false).await;
    let dirty = git(&vfs, &["status", "--porcelain"]).await;
    assert_eq!(text(&dirty.stdout), " M root.txt\n?? new.txt\n");

    let out = git(&vfs, &["add", "root.txt", "new.txt"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let index = git(&vfs, &["ls-files", "--stage"]).await;
    assert_eq!(
        text(&index.stdout),
        "100644 d00491fd7e5bb6fa28c517a0bb32b8b506539d4d 0\ta/b/c/deep.txt\n\
         100644 0cfbf08886fca9a91cb753ec8734c84fcbe52c9f 0\ta/top.txt\n\
         100644 8ba3a16384aacc37d01564b28401755ce8053f51 0\tnew.txt\n\
         100644 7a7b8aae67a0c255a7b13641fb7b630963842409 0\troot.txt\n"
    );
    let tree = git(&vfs, &["write-tree"]).await;
    assert_eq!(
        text(&tree.stdout),
        "baac38ad086d955f079de720551e7f90b09a78a0\n"
    );
    let blob = git(&vfs, &["hash-object", "root.txt"]).await;
    assert_eq!(
        text(&blob.stdout),
        "7a7b8aae67a0c255a7b13641fb7b630963842409\n"
    );

    let subjects = git(&vfs, &["log", "--format=%s"]).await;
    assert_eq!(text(&subjects.stdout), "first\n");
}

/// `git diff` (worktree) on a modified tracked file must match host git's
/// unified-diff output byte-for-byte.
#[tokio::test]
async fn git_diff_worktree_modify_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    put(&vfs, "/work/f.txt", b"one\ntwo\nthree\n", false).await;
    git(&vfs, &["add", "f.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Modify: change "two" to "TWO"
    put(&vfs, "/work/f.txt", b"one\nTWO\nthree\n", false).await;

    let out = git(&vfs, &["diff"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/f.txt b/f.txt
index 4cb29ea..ddc897f 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@
 one
-two
+TWO
 three
";
    assert_eq!(text(&out.stdout), expected);
}

/// `git diff` (worktree) on a deleted tracked file.
#[tokio::test]
async fn git_diff_worktree_delete_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    put(&vfs, "/work/f.txt", b"alpha\nbeta\n", false).await;
    put(&vfs, "/work/g.txt", b"gamma\ndelta\n", false).await;
    git(&vfs, &["add", "f.txt", "g.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Delete f.txt from the worktree
    vfs.rm(Path::new("/work/f.txt"), false).await.unwrap();

    let out = git(&vfs, &["diff"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/f.txt b/f.txt
deleted file mode 100644
index fbbee86..0000000
--- a/f.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-alpha
-beta
";
    assert_eq!(text(&out.stdout), expected);
}

/// `git diff --cached` on a staged modification.
#[tokio::test]
async fn git_diff_cached_modify_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    put(&vfs, "/work/f.txt", b"1\n2\n3\n", false).await;
    git(&vfs, &["add", "f.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Modify and stage
    put(&vfs, "/work/f.txt", b"1\nX\n3\n", false).await;
    git(&vfs, &["add", "f.txt"]).await;

    let out = git(&vfs, &["diff", "--cached"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/f.txt b/f.txt
index 01e79c3..f081db6 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@
 1
-2
+X
 3
";
    assert_eq!(text(&out.stdout), expected);
}

/// `git diff` on a multi-hunk change (two separated modifications).
#[tokio::test]
async fn git_diff_multi_hunk_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    let content: Vec<String> = (1..=9).map(|i| format!("{i}\n")).collect();
    put(&vfs, "/work/f.txt", content.concat().as_bytes(), false).await;
    git(&vfs, &["add", "f.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Change line 2 and line 8
    let mut new_content: Vec<String> = (1..=9).map(|i| format!("{i}\n")).collect();
    new_content[1] = "2X\n".to_string();
    new_content[7] = "8X\n".to_string();
    put(&vfs, "/work/f.txt", new_content.concat().as_bytes(), false).await;

    let out = git(&vfs, &["diff"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/f.txt b/f.txt
index 0719398..abbba22 100644
--- a/f.txt
+++ b/f.txt
@@ -1,9 +1,9 @@
 1
-2
+2X
 3
 4
 5
 6
 7
-8
+8X
 9
";
    assert_eq!(text(&out.stdout), expected);
}

/// `git diff` on a new (untracked) file shows nothing; on a staged new file
/// `git diff --cached` shows the full addition.
#[tokio::test]
async fn git_diff_new_file_cached_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    put(&vfs, "/work/seed.txt", b"seed\n", false).await;
    git(&vfs, &["add", "seed.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Create and stage a new file
    put(&vfs, "/work/n.txt", b"hello\nworld\n", false).await;
    git(&vfs, &["add", "n.txt"]).await;

    // Plain `git diff` (worktree) should show nothing (no worktree changes)
    let out = git(&vfs, &["diff"]).await;
    assert_eq!(out.exit_code, 0);
    assert_eq!(text(&out.stdout), "");

    // `git diff --cached` should show the new file
    let out = git(&vfs, &["diff", "--cached"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/n.txt b/n.txt
new file mode 100644
index 0000000..94954ab
--- /dev/null
+++ b/n.txt
@@ -0,0 +1,2 @@
+hello
+world
";
    assert_eq!(text(&out.stdout), expected);
}

/// `git diff` with a duplicated line: the new file appends a copy of an
/// existing line. The LCS must pair the original with the *earliest* copy so
/// the diff shows a pure append, matching host git's hunk alignment.
#[tokio::test]
async fn git_diff_duplicate_line_append_matches_host_git() {
    let vfs = vfs();
    vfs.mkdir(Path::new("/work"), 0o755).await.unwrap();
    git(&vfs, &["init", "-q"]).await;
    put(&vfs, "/work/f.txt", b"3\nmore\n", false).await;
    git(&vfs, &["add", "f.txt"]).await;
    git(
        &vfs,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-m",
            "init",
        ],
    )
    .await;
    // Append a duplicate of "more"
    put(&vfs, "/work/f.txt", b"3\nmore\nmore\n", false).await;

    let out = git(&vfs, &["diff"]).await;
    assert_eq!(out.exit_code, 0, "{}", text(&out.stderr));
    let expected = "\
diff --git a/f.txt b/f.txt
index 7a7b8aa..af0c6c8 100644
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,3 @@
 3
 more
+more
";
    assert_eq!(text(&out.stdout), expected);
}
