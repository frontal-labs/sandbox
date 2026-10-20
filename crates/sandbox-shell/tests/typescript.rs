//! `node FILE.ts` runs TypeScript by erasing its types (PLAN.md §9 M1). Each case
//! writes a `.ts` file into the VFS and runs it, so the path is the one an agent uses.

use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::Vfs;
use std::sync::Arc;

async fn run(script: &str) -> (String, String, i32) {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut shell = Shell::new(fs, ShellConfig::default());
    let out = shell.run(script).await.expect("the shell script parses");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.exit_code,
    )
}

#[tokio::test]
async fn type_annotations_are_erased_before_running() {
    let (stdout, stderr, status) =
        run("printf 'const x: number = 2;\\nconsole.log(x * 3);\\n' > a.ts; node a.ts").await;
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "6\n");
}

#[tokio::test]
async fn interfaces_generics_and_casts_run() {
    let (stdout, stderr, status) = run(
        "printf 'interface P { a: string }\\nconst p: P = { a: \"hi\" } as P;\\nfunction id<T>(v: T): T { return v; }\\nconsole.log(id<string>(p.a));\\n' > b.ts; node b.ts",
    )
    .await;
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "hi\n");
}

#[tokio::test]
async fn a_typescript_syntax_error_is_reported_and_nothing_runs() {
    let (stdout, stderr, status) =
        run("printf 'const = ;\\nconsole.log(1);\\n' > c.ts; node c.ts").await;
    assert_eq!(status, 1);
    assert_eq!(stdout, "");
    assert!(stderr.contains("TypeScript syntax error"), "{stderr}");
}

#[tokio::test]
async fn a_plain_js_file_still_runs_unchanged() {
    let (stdout, _, status) = run("printf 'console.log(\"plain\");\\n' > d.js; node d.js").await;
    assert_eq!(status, 0);
    assert_eq!(stdout, "plain\n");
}

#[tokio::test]
async fn enums_and_parameter_properties_are_lowered() {
    let (stdout, stderr, status) = run(
        "printf 'enum E { A = 1 }\\nclass P { constructor(private x: number) {} get v(): number { return this.x; } }\\nconsole.log(E.A, new P(4).v);\\n' > f.ts; node f.ts",
    )
    .await;
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "1 4\n");
}

#[tokio::test]
async fn types_are_not_checked_so_a_type_error_still_runs() {
    // The transform erases types without checking them. The output is the value the
    // code computes, which is what an agent's script expects from `node`.
    let (stdout, stderr, status) =
        run("printf 'const s: number = \"text\";\\nconsole.log(s);\\n' > g.ts; node g.ts").await;
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(stdout, "text\n");
}
