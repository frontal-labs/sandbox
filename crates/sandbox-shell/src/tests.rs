use crate::interpreter::{Shell, ShellConfig};
use crate::parser::parse;
use sandbox_cas::MemStore;
use sandbox_types::{hash::ContentHash, ErrorCode};
use sandbox_vfs::Vfs;
use std::sync::Arc;

fn shell() -> Shell {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    Shell::new(fs, ShellConfig::default())
}

fn simple(command: &crate::ast::Command) -> &crate::ast::SimpleCommand {
    match command {
        crate::ast::Command::Simple(simple) => simple,
        other => panic!("expected a simple command, got {other:?}"),
    }
}

async fn run(shell: &mut Shell, script: &str) -> (i32, String, String) {
    let out = shell.run(script).await.unwrap();
    (
        out.exit_code,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn parses_quotes_and_operators() {
    let script = parse(r#"echo "a  b" 'c d' | cat && false || true; ls"#).unwrap();
    assert_eq!(script.items.len(), 4);
    let first = simple(&script.items[0].pipeline.stages[0]);
    let words: Vec<String> = first.words.iter().map(|w| w.text()).collect();
    assert_eq!(words, vec!["echo", "a  b", "c d"]);
    assert_eq!(script.items[0].pipeline.stages.len(), 2);
}

#[test]
fn rejects_malformed_scripts() {
    for bad in [
        "echo 'unterminated",
        "echo \"unterminated",
        "&& echo x",
        "echo x &&",
        "echo x |",
        "cat >",
        "echo x &",
        "echo x 3> f",
    ] {
        let err = parse(bad).expect_err(bad);
        assert_eq!(err.code, ErrorCode::ShellError, "{bad}");
    }
}

#[test]
fn comments_and_empty_scripts_are_fine() {
    assert!(parse("").unwrap().items.is_empty());
    assert!(parse("# only a comment").unwrap().items.is_empty());
    assert_eq!(parse("echo hi # trailing").unwrap().items.len(), 1);
}

#[tokio::test]
async fn pipe_and_redirect_round_trip_through_the_vfs() {
    let mut sh = shell();
    let (code, out, _) = run(&mut sh, "echo hi | cat > /a && cat /a").await;
    assert_eq!((code, out.as_str()), (0, "hi\n"));
}

#[tokio::test]
async fn and_or_and_seq_follow_exit_status() {
    let mut sh = shell();
    let (_, out, _) = run(&mut sh, "false && echo no || echo yes").await;
    assert_eq!(out, "yes\n");
    let (_, out, _) = run(&mut sh, "true && echo a; false; echo b").await;
    assert_eq!(out, "a\nb\n");
}

#[tokio::test]
async fn append_and_input_redirects() {
    let mut sh = shell();
    run(&mut sh, "echo a > /f").await;
    run(&mut sh, "echo b >> /f").await;
    let (_, out, _) = run(&mut sh, "cat < /f").await;
    assert_eq!(out, "a\nb\n");
}

#[tokio::test]
async fn quoting_keeps_spaces_and_escapes_work() {
    let mut sh = shell();
    let (_, out, _) = run(&mut sh, r#"echo "x  y" 'p q' a\ b"#).await;
    assert_eq!(out, "x  y p q a b\n");
}

#[tokio::test]
async fn unknown_command_exits_127_with_a_clear_message() {
    let mut sh = shell();
    let (code, _, err) = run(&mut sh, "python -c 1").await;
    assert_eq!(code, 127);
    assert!(err.contains("command not found"), "{err}");
}

#[tokio::test]
async fn cd_pwd_and_dot_dot_stay_under_root() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /a/b").await;
    run(&mut sh, "cd /a/b").await;
    assert_eq!(run(&mut sh, "pwd").await.1, "/a/b\n");
    run(&mut sh, "cd ../..").await;
    assert_eq!(run(&mut sh, "pwd").await.1, "/\n");
    run(&mut sh, "cd ../../..").await;
    assert_eq!(run(&mut sh, "pwd").await.1, "/\n");
}

#[tokio::test]
async fn file_builtins_work_end_to_end() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /d/sub && touch /d/a /d/b").await;
    assert_eq!(run(&mut sh, "ls /d").await.1, "a\nb\nsub\n");
    run(&mut sh, "mv /d/a /d/sub").await;
    assert_eq!(run(&mut sh, "ls /d/sub").await.1, "a\n");
    run(&mut sh, "cp /d/sub/a /d/c").await;
    assert_eq!(run(&mut sh, "ls /d").await.1, "b\nc\nsub\n");
    run(&mut sh, "echo 1 2 3 > /n").await;
    assert_eq!(run(&mut sh, "wc -w /n").await.1.trim(), "3 /n");
    run(&mut sh, "rm -r /d").await;
    assert_eq!(run(&mut sh, "ls /").await.1, "n\n");
}

#[tokio::test]
async fn head_limits_lines() {
    let mut sh = shell();
    run(&mut sh, "echo 1 > /x && echo 2 >> /x && echo 3 >> /x").await;
    assert_eq!(run(&mut sh, "head -n 2 /x").await.1, "1\n2\n");
}

#[tokio::test]
async fn errors_are_reported_on_stderr_with_exit_1() {
    let mut sh = shell();
    let (code, _, err) = run(&mut sh, "cat /missing").await;
    assert_eq!(code, 1);
    assert!(
        err.contains("/missing") && err.contains("No such file"),
        "{err}"
    );

    run(&mut sh, "mkdir /dir").await;
    let (code, _, err) = run(&mut sh, "rm /dir").await;
    assert_eq!(code, 1);
    assert!(err.contains("is a directory"), "{err}");
}

#[tokio::test]
async fn run_argv_passes_stdin_through() {
    let mut sh = shell();
    let out = sh
        .run_argv(&["cat".to_string()], b"from stdin")
        .await
        .unwrap();
    assert_eq!(out.stdout, b"from stdin");
    assert_eq!(out.exit_code, 0);
}

#[tokio::test]
async fn node_runs_inline_code_and_files() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let mut sh = shell();
    let (code, out, _) = run(&mut sh, r#"node -e 'console.log(6 * 7)'"#).await;
    assert_eq!((code, out.as_str()), (0, "42\n"));

    run(&mut sh, "echo 'console.log(\"from file\")' > /s.js").await;
    assert_eq!(run(&mut sh, "node /s.js").await.1, "from file\n");
    assert_eq!(run(&mut sh, "echo 'console.log(1)' | node").await.1, "1\n");
}

#[tokio::test]
async fn node_reports_script_errors_with_exit_1() {
    let _ = sandbox_runtime::install_in_process_script_runner();
    let mut sh = shell();
    let (code, _, err) = run(&mut sh, "node -e 'throw new Error(\"nope\")'").await;
    assert_eq!(code, 1);
    assert!(err.contains("nope"), "{err}");
}

const HELLO_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "wasm says hi\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 13))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 100)))
    (call $proc_exit (i32.const 0))))
"#;

#[tokio::test]
async fn wasm_builtin_runs_a_module_from_the_vfs() {
    let mut sh = shell();
    sh.write_file("/hello.wat", HELLO_WAT.as_bytes(), false)
        .await
        .unwrap();
    let (code, out, err) = run(&mut sh, "wasm /hello.wat").await;
    assert_eq!((code, out.as_str()), (0, "wasm says hi\n"), "stderr: {err}");

    let (code, _, err) = run(&mut sh, "wasm /missing.wasm").await;
    assert_eq!(code, 1);
    assert!(err.contains("No such file"), "{err}");
}

#[tokio::test]
async fn variables_expand_and_persist_across_calls() {
    let mut sh = shell();
    run(&mut sh, "NAME=world").await;
    assert_eq!(
        run(&mut sh, r#"echo "hello $NAME" ${NAME}!"#).await.1,
        "hello world world!\n"
    );
    assert_eq!(run(&mut sh, "echo '$NAME'").await.1, "$NAME\n");
    assert_eq!(run(&mut sh, "echo $MISSING.").await.1, ".\n");
}

#[tokio::test]
async fn prefix_assignments_last_only_for_their_command() {
    let mut sh = shell();
    run(&mut sh, "X=outer").await;
    // Expansion happens before the prefix assignment, as in POSIX sh, and the
    // assignment is undone once the command finishes.
    assert_eq!(run(&mut sh, "X=inner echo $X").await.1, "outer\n");
    run(&mut sh, "X=inner echo hi").await;
    assert_eq!(run(&mut sh, "echo $X").await.1, "outer\n");
}

#[tokio::test]
async fn exit_status_is_visible_as_dollar_question() {
    let mut sh = shell();
    run(&mut sh, "false").await;
    assert_eq!(run(&mut sh, "echo $?").await.1, "1\n");
    run(&mut sh, "true").await;
    assert_eq!(run(&mut sh, "echo $?").await.1, "0\n");
}

#[tokio::test]
async fn globs_expand_sorted_and_skip_hidden_files() {
    let mut sh = shell();
    run(
        &mut sh,
        "mkdir /g && touch /g/b.txt /g/a.txt /g/c.md /g/.hidden.txt",
    )
    .await;
    assert_eq!(run(&mut sh, "echo /g/*.txt").await.1, "/g/a.txt /g/b.txt\n");
    assert_eq!(run(&mut sh, "echo /g/?.md").await.1, "/g/c.md\n");
    assert_eq!(run(&mut sh, "echo /g/.h*").await.1, "/g/.hidden.txt\n");
    // A pattern with no match is passed through unchanged, as in POSIX sh.
    assert_eq!(run(&mut sh, "echo /g/*.zip").await.1, "/g/*.zip\n");
    // Quoted glob characters are literal.
    assert_eq!(run(&mut sh, "echo '/g/*.txt'").await.1, "/g/*.txt\n");
}

#[tokio::test]
async fn globs_are_relative_to_the_working_directory() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /p && touch /p/x.log /p/y.log && cd /p").await;
    assert_eq!(run(&mut sh, "echo *.log").await.1, "x.log y.log\n");
}

#[tokio::test]
async fn output_redirect_without_a_command_creates_the_file() {
    let mut sh = shell();
    run(&mut sh, "echo old > /t").await;
    run(&mut sh, "> /t").await;
    assert_eq!(run(&mut sh, "cat /t").await.1, "");
}

#[tokio::test]
async fn wasm_tools_on_path_run_as_commands() {
    let mut sh = shell();
    run(&mut sh, "mkdir /bin /tools").await;
    sh.write_file("/bin/hello", HELLO_WAT.as_bytes(), false)
        .await
        .unwrap();
    let (code, out, err) = run(&mut sh, "hello").await;
    assert_eq!((code, out.as_str()), (0, "wasm says hi\n"), "stderr: {err}");

    // PATH decides where tools are looked up.
    sh.write_file("/tools/hello", HELLO_WAT.as_bytes(), false)
        .await
        .unwrap();
    run(&mut sh, "PATH=/tools").await;
    assert_eq!(run(&mut sh, "hello").await.0, 0);
    let (code, _, err) = run(&mut sh, "hello2").await;
    assert_eq!(code, 127);
    assert!(err.contains("command not found"), "{err}");
}

#[test]
fn redirect_only_commands_parse() {
    let script = parse("> /created").unwrap();
    let command = simple(&script.items[0].pipeline.stages[0]);
    assert!(command.words.is_empty());
    assert_eq!(command.redirects.len(), 1);
}

const FIXTURE: &[u8] = include_bytes!("../../sandbox-wasm/fixtures/wasi_tool.wasm");

#[tokio::test]
async fn wasi_tool_fixture_reads_files_relative_to_the_working_directory() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /bin /w").await;
    sh.write_file("/bin/wasi_tool", FIXTURE, false)
        .await
        .unwrap();
    run(&mut sh, "echo inside > /w/note && cd /w").await;
    let (code, out, err) = run(&mut sh, "wasi_tool cat note").await;
    assert_eq!((code, out.as_str()), (0, "inside\n"), "stderr: {err}");

    // Absolute paths and `..` cannot reach outside the working directory.
    let (code, out, _) = run(&mut sh, "wasi_tool cat /bin/wasi_tool").await;
    assert_eq!(code, 1);
    assert!(out.is_empty());
}

#[tokio::test]
async fn registry_entries_pin_hashes_and_apply_limits() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /etc /opt").await;
    sh.write_file("/opt/wasi_tool.wasm", FIXTURE, false)
        .await
        .unwrap();
    let pinned = sandbox_types::hash::ContentHash::new(FIXTURE).to_hex();
    let manifest = format!(
        "[[tool]]\nname = \"wt\"\npath = \"/opt/wasi_tool.wasm\"\nversion = \"1.2.3\"\nhash = \"{pinned}\"\nfuel = 1\n"
    );
    sh.write_file("/etc/tools.toml", manifest.as_bytes(), false)
        .await
        .unwrap();

    // The 1-instruction fuel limit stops the tool before it does anything useful.
    let (code, _, err) = run(&mut sh, "wt args").await;
    assert_eq!(code, 1, "{err}");

    assert_eq!(
        run(&mut sh, "tools").await.1,
        "wt 1.2.3 /opt/wasi_tool.wasm\n"
    );

    // Tampering with the module after it was pinned is refused before it runs.
    sh.write_file("/opt/wasi_tool.wasm", b"tampered", false)
        .await
        .unwrap();
    let (code, _, err) = run(&mut sh, "wt args").await;
    assert_eq!(code, 126);
    assert!(err.contains("pinned hash"), "{err}");
}

#[tokio::test]
async fn a_malformed_registry_fails_closed() {
    let mut sh = shell();
    run(&mut sh, "mkdir -p /etc").await;
    sh.write_file("/etc/tools.toml", b"[[tool]]\nname = 3", false)
        .await
        .unwrap();
    let (code, _, err) = run(&mut sh, "anything").await;
    assert_eq!(code, 126);
    assert!(err.contains("tools manifest"), "{err}");
}

#[tokio::test]
async fn if_elif_else_pick_one_branch() {
    let mut sh = shell();
    let script = |cond: &str| {
        format!("if {cond}; then echo then; elif false; then echo elif; else echo else; fi")
    };
    assert_eq!(run(&mut sh, &script("true")).await.1, "then\n");
    assert_eq!(run(&mut sh, &script("false")).await.1, "else\n");
    assert_eq!(
        run(&mut sh, "if false; then echo a; elif true; then echo b; fi")
            .await
            .1,
        "b\n"
    );
    // No branch taken and no else: the status is 0.
    assert_eq!(
        run(&mut sh, "if false; then echo a; fi; echo $?").await.1,
        "0\n"
    );
}

#[tokio::test]
async fn test_and_bracket_builtins_drive_conditions() {
    let mut sh = shell();
    run(&mut sh, "mkdir /t && touch /t/f").await;
    assert_eq!(run(&mut sh, "[ -f /t/f ] && echo file").await.1, "file\n");
    assert_eq!(
        run(&mut sh, "[ -d /t/f ] || echo notdir").await.1,
        "notdir\n"
    );
    assert_eq!(run(&mut sh, "test 3 -lt 10 && echo lt").await.1, "lt\n");
    assert_eq!(
        run(&mut sh, "[ abc = abc ] && [ -n x ] && echo both")
            .await
            .1,
        "both\n"
    );
    assert_eq!(run(&mut sh, "[ ! -z '' ] || echo empty").await.1, "empty\n");
    let (code, _, err) = run(&mut sh, "[ 1 -eq x ]").await;
    assert_eq!(code, 2);
    assert!(err.contains("integer expression expected"), "{err}");
}

#[tokio::test]
async fn while_and_until_loop_with_break_and_continue() {
    let mut sh = shell();
    run(&mut sh, "N=0").await;
    let out = run(
        &mut sh,
        "while [ $N = 0 ]; do echo once; N=1; break; done; echo done",
    )
    .await
    .1;
    assert_eq!(out, "once\ndone\n");
    // N is 1 now, so `until` (run while the condition fails) never runs its body.
    let out = run(&mut sh, "until [ $N = 1 ]; do echo never; done; echo after")
        .await
        .1;
    assert_eq!(out, "after\n");
}

#[tokio::test]
async fn loop_counters_and_continue_skip_iterations() {
    let mut sh = shell();
    run(&mut sh, "i=0").await;
    // `$((...))` is not supported, so count with a file-free trick: nested ifs over a list.
    let out = run(
        &mut sh,
        "for x in 1 2 3 4 5; do if [ $x = 3 ]; then continue; fi; if [ $x = 5 ]; then break; fi; echo $x; done",
    )
    .await
    .1;
    assert_eq!(out, "1\n2\n4\n");

    let out = run(&mut sh, "until false; do echo once; break; done")
        .await
        .1;
    assert_eq!(out, "once\n");
}

#[tokio::test]
async fn nested_loops_break_by_count() {
    let mut sh = shell();
    let out = run(
        &mut sh,
        "for a in 1 2; do for b in x y; do echo $a$b; break 2; done; done; echo end",
    )
    .await
    .1;
    assert_eq!(out, "1x\nend\n");
}

#[tokio::test]
async fn for_loops_expand_words_and_globs() {
    let mut sh = shell();
    run(&mut sh, "mkdir /fl && touch /fl/b.log /fl/a.log").await;
    let out = run(
        &mut sh,
        "cd /fl; for f in *.log plain \"quoted w\"; do echo $f; done",
    )
    .await
    .1;
    assert_eq!(out, "a.log\nb.log\nplain\nquoted w\n");
    assert_eq!(
        run(&mut sh, "for z in; do echo never; done; echo ok")
            .await
            .1,
        "ok\n"
    );
}

#[tokio::test]
async fn functions_take_arguments_return_status_and_see_globals() {
    let mut sh = shell();
    run(&mut sh, "greet() { echo hello $1 and $#; return 3; }").await;
    run(&mut sh, "G=global").await;
    let (code, out, _) = run(&mut sh, "greet world").await;
    assert_eq!((code, out.as_str()), (3, "hello world and 1\n"));
    // Positional parameters are restored after the call.
    assert_eq!(run(&mut sh, "echo ${#}").await.1, "0\n");
    run(&mut sh, "show() { echo $G; }").await;
    assert_eq!(run(&mut sh, "show").await.1, "global\n");
    // A function defined inside a function is callable afterwards.
    run(&mut sh, "outer() { inner() { echo deep; }; inner; }").await;
    assert_eq!(run(&mut sh, "outer").await.1, "deep\n");
}

#[tokio::test]
async fn functions_can_be_piped_into_and_out_of() {
    let mut sh = shell();
    run(&mut sh, "upper() { cat; }").await;
    assert_eq!(run(&mut sh, "echo shout | upper").await.1, "shout\n");
    assert_eq!(run(&mut sh, "echo a | { cat; echo b; }").await.1, "a\nb\n");
}

#[tokio::test]
async fn command_substitution_in_arguments_and_assignments() {
    let mut sh = shell();
    assert_eq!(run(&mut sh, "echo $(echo inner)").await.1, "inner\n");
    assert_eq!(run(&mut sh, "echo `echo back`").await.1, "back\n");
    assert_eq!(run(&mut sh, "echo \"[$(echo a b)]\"").await.1, "[a b]\n");
    assert_eq!(run(&mut sh, "X=$(echo value); echo $X").await.1, "value\n");
    assert_eq!(run(&mut sh, "echo $(echo $(echo deep))").await.1, "deep\n");
    // Trailing newlines are removed, interior ones are kept.
    assert_eq!(run(&mut sh, "echo \"$(echo a; echo b)\"").await.1, "a\nb\n");
}

#[tokio::test]
async fn substitutions_see_but_do_not_change_the_shell() {
    let mut sh = shell();
    run(&mut sh, "mkdir /sub && cd /sub").await;
    run(&mut sh, "V=outer").await;
    assert_eq!(run(&mut sh, "echo $(cd / ; pwd)").await.1, "/\n");
    assert_eq!(run(&mut sh, "pwd").await.1, "/sub\n");
    assert_eq!(run(&mut sh, "echo $(V=inner; echo $V)").await.1, "inner\n");
    assert_eq!(run(&mut sh, "echo $V").await.1, "outer\n");
}

#[tokio::test]
async fn subshells_are_isolated_copies() {
    let mut sh = shell();
    run(&mut sh, "mkdir /iso && cd /iso && A=1").await;
    assert_eq!(run(&mut sh, "(cd / ; A=2; pwd; echo $A)").await.1, "/\n2\n");
    assert_eq!(run(&mut sh, "pwd; echo $A").await.1, "/iso\n1\n");
}

#[tokio::test]
async fn runaway_loops_hit_the_step_budget() {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut sh = Shell::new(
        fs,
        ShellConfig {
            max_steps: 500,
            ..ShellConfig::default()
        },
    );
    let (code, _, err) = run(&mut sh, "while true; do :; done; echo unreachable").await;
    assert_eq!(code, 1);
    assert!(err.contains("step limit"), "{err}");
    // The next run starts with a fresh budget.
    assert_eq!(run(&mut sh, "echo fresh").await.1, "fresh\n");
}

#[tokio::test]
async fn runaway_output_hits_the_output_budget() {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut sh = Shell::new(
        fs,
        ShellConfig {
            max_output_bytes: 1000,
            ..ShellConfig::default()
        },
    );
    let (code, out, err) = run(&mut sh, "while true; do echo spam; done").await;
    assert_eq!(code, 1);
    assert!(out.len() <= 1000, "stdout grew to {}", out.len());
    assert!(err.contains("output limit"), "{err}");
}

#[tokio::test]
async fn unclosed_or_stray_compound_commands_are_syntax_errors() {
    for bad in [
        "if true; then echo x",
        "fi",
        "while true; do echo x",
        "for in a b; do echo; done",
        "for x a; do echo; done",
        "echo x &&",
        "{ echo x",
        "(echo x",
        "echo $(echo x",
        "echo `unclosed",
    ] {
        let err = parse(bad).expect_err(bad);
        assert_eq!(err.code, ErrorCode::ShellError, "{bad}");
    }
}

#[tokio::test]
async fn export_and_unset_manage_variables() {
    let mut sh = shell();
    run(&mut sh, "export GREETING=hi").await;
    assert_eq!(run(&mut sh, "echo $GREETING").await.1, "hi\n");
    run(&mut sh, "unset GREETING").await;
    assert_eq!(run(&mut sh, "echo [$GREETING]").await.1, "[]\n");
    let (code, _, err) = run(&mut sh, "export 1bad=x").await;
    assert_eq!(code, 1);
    assert!(err.contains("not a valid name"), "{err}");
}

#[tokio::test]
async fn sandbox_info_prints_the_environment_file() {
    let mut sh = shell();
    run(&mut sh, "mkdir /etc; mkdir /etc/agent").await;
    run(&mut sh, "echo '{\"schema\":1}' > /etc/agent/env.json").await;
    assert_eq!(run(&mut sh, "sandbox info").await.1, "{\"schema\":1}\n");
    assert_eq!(
        run(&mut sh, "sandbox info --json").await.1,
        "{\"schema\":1}\n"
    );
}

#[tokio::test]
async fn sandbox_refuses_unknown_commands_and_options() {
    let mut sh = shell();
    run(&mut sh, "mkdir /etc; mkdir /etc/agent").await;
    run(&mut sh, "echo '{}' > /etc/agent/env.json").await;
    let (code, _, err) = run(&mut sh, "sandbox").await;
    assert_eq!(code, 2);
    assert!(err.contains("usage: sandbox info"), "{err}");
    let (code, _, err) = run(&mut sh, "sandbox nap").await;
    assert_eq!(code, 2);
    assert!(err.contains("unknown command 'nap'"), "{err}");
    let (code, _, err) = run(&mut sh, "sandbox info --yaml").await;
    assert_eq!(code, 2);
    assert!(err.contains("unknown option '--yaml'"), "{err}");
}

#[tokio::test]
async fn sandbox_info_reports_a_missing_file_as_an_error() {
    let mut sh = shell();
    let (code, _, err) = run(&mut sh, "sandbox info").await;
    assert_eq!(code, 1);
    assert!(err.contains("sandbox info: /etc/agent/env.json"), "{err}");
}
