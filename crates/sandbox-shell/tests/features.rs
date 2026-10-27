//! Behaviour of the shell features that the conformance corpus does not isolate:
//! stream routing, exit scoping, failure statuses, and refusals of syntax the shell
//! does not support. Each expectation was checked against GNU bash 5.3, except the
//! refusals, which have no bash equivalent: the sandbox reports them as errors.

use sandbox_cas::MemStore;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_types::hash::ContentHash;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::Vfs;
use std::sync::Arc;

struct Outcome {
    stdout: String,
    stderr: String,
    status: i32,
}

async fn run(script: &str) -> Result<Outcome, SandboxError> {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let mut shell = Shell::new(fs, ShellConfig::default());
    let out = shell.run(script).await?;
    Ok(Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status: out.exit_code,
    })
}

async fn stdout_of(script: &str) -> String {
    run(script).await.unwrap().stdout
}

#[tokio::test]
async fn stderr_can_be_merged_into_stdout_in_order() {
    let out = run("cat nope > o.txt 2>&1; cat o.txt").await.unwrap();
    assert!(out.stdout.contains("nope"), "{:?}", out.stdout);
    assert!(out.stderr.is_empty(), "{:?}", out.stderr);
}

#[tokio::test]
async fn stdout_can_be_sent_to_stderr() {
    let out = run("echo err >&2").await.unwrap();
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "err\n");
}

#[tokio::test]
async fn dev_null_discards_output() {
    let out = run("echo hi 2>/dev/null >/dev/null; echo ok")
        .await
        .unwrap();
    assert_eq!(out.stdout, "ok\n");
    assert_eq!(out.stderr, "");
}

#[tokio::test]
async fn descriptors_above_two_are_refused_at_parse_time() {
    let err = run("echo x 3> f").await.err().unwrap();
    assert_eq!(err.code, ErrorCode::ShellError);
}

#[tokio::test]
async fn exit_ends_the_run_with_its_status() {
    let out = run("echo a; exit 3; echo b").await.unwrap();
    assert_eq!(out.stdout, "a\n");
    assert_eq!(out.status, 3);
}

#[tokio::test]
async fn exit_in_a_subshell_ends_only_the_subshell() {
    assert_eq!(stdout_of("(exit 4); echo $?").await, "4\n");
}

#[tokio::test]
async fn exit_inside_a_loop_stops_the_loop() {
    assert_eq!(
        stdout_of("for i in 1 2 3; do echo $i; exit 0; done; echo after").await,
        "1\n"
    );
}

#[tokio::test]
async fn an_arithmetic_error_ends_the_run_with_status_one() {
    // bash: the command does not run, and the rest of the script does not either.
    let out = run("echo $((1/0)); echo after").await.unwrap();
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
    assert!(out.stderr.contains("division by 0"), "{:?}", out.stderr);
}

#[tokio::test]
async fn unquoted_expansions_split_on_ifs_and_quoted_ones_do_not() {
    assert_eq!(
        stdout_of("x='a  b'; for w in $x; do echo [$w]; done").await,
        "[a]\n[b]\n"
    );
    // `echo [$w]` would split the unquoted `$w` again, so print it with `printf`.
    assert_eq!(
        stdout_of("x='a  b'; for w in \"$x\"; do printf '[%s]\\n' \"$w\"; done").await,
        "[a  b]\n"
    );
}

#[tokio::test]
async fn at_keeps_each_positional_parameter_as_one_field() {
    assert_eq!(
        stdout_of("f() { for a in \"$@\"; do echo \"<$a>\"; done; }; f 'x y' z").await,
        "<x y>\n<z>\n"
    );
    // With no parameters, "$@" is no field at all.
    assert_eq!(
        stdout_of("f() { for a in \"$@\"; do echo \"<$a>\"; done; echo end; }; f").await,
        "end\n"
    );
}

#[tokio::test]
async fn a_loop_redirection_applies_to_every_iteration() {
    assert_eq!(
        stdout_of("for i in 1 2; do echo $i; done > f; cat f").await,
        "1\n2\n"
    );
}

#[tokio::test]
async fn negation_inverts_the_status() {
    assert_eq!(stdout_of("! false; echo $?").await, "0\n");
    assert_eq!(stdout_of("! true; echo $?").await, "1\n");
}

#[tokio::test]
async fn case_without_a_match_has_status_zero() {
    assert_eq!(
        stdout_of("false; case x in y) echo no;; esac; echo $?").await,
        "0\n"
    );
}

#[tokio::test]
async fn read_at_end_of_input_fails_but_still_assigns() {
    assert_eq!(
        stdout_of("printf 'x' | { read a; echo $? $a; }").await,
        "1 x\n"
    );
}

#[tokio::test]
async fn grep_reports_no_match_as_one_and_error_as_two() {
    assert_eq!(stdout_of("echo a > f; grep b f; echo $?").await, "1\n");
    assert_eq!(stdout_of("grep a nosuch 2>/dev/null; echo $?").await, "2\n");
    assert_eq!(stdout_of("echo a > f; grep -q a f; echo $?").await, "0\n");
}

#[tokio::test]
async fn grep_takes_regular_expressions_and_fixed_strings() {
    assert_eq!(
        stdout_of("printf 'a.c\\nabc\\n' | grep 'a.c'").await,
        "a.c\nabc\n"
    );
    assert_eq!(
        stdout_of("printf 'a.c\\nabc\\n' | grep -F 'a.c'").await,
        "a.c\n"
    );
    assert_eq!(stdout_of("printf 'Hi\\nno\\n' | grep -i hi").await, "Hi\n");
}

#[tokio::test]
async fn printf_reports_a_bad_number_as_zero_with_status_one() {
    // bash prints the 0 and then the status: `01`.
    assert_eq!(
        stdout_of("printf '%d' x 2>/dev/null; echo $?").await,
        "01\n"
    );
}

#[tokio::test]
async fn tail_takes_the_last_lines_of_short_input() {
    assert_eq!(stdout_of("printf 'x\\ny\\n' | tail -n 5").await, "x\ny\n");
}

#[tokio::test]
async fn type_reports_builtins_and_missing_names() {
    assert_eq!(stdout_of("type echo").await, "echo is a shell builtin\n");
    assert_eq!(
        stdout_of("type nosuchthing 2>/dev/null; echo $?").await,
        "1\n"
    );
}

#[tokio::test]
async fn an_unsupported_operator_is_refused_not_guessed() {
    // `${x:?msg}` (error when unset) is not supported. It must be refused, not run.
    let err = run("x=; echo ${x:?no}").await.err().unwrap();
    assert_eq!(err.code, ErrorCode::ShellError);
}

#[tokio::test]
async fn a_trailing_backslash_is_a_literal_backslash() {
    // bash: `echo x\` prints `x\`.
    assert_eq!(stdout_of("echo x\\").await, "x\\\n");
}

#[tokio::test]
async fn an_assignment_reports_the_status_of_its_substitution() {
    assert_eq!(stdout_of("x=$(false); echo $?").await, "1\n");
    assert_eq!(stdout_of("x=$(true); echo $?").await, "0\n");
}

#[tokio::test]
async fn pipeline_stages_read_the_input_of_an_enclosing_loop_only_when_they_read_stdin() {
    // `echo` does not read stdin, so the loop still sees every line.
    assert_eq!(
        stdout_of("printf 'a\\nb\\n' | while read line; do echo got $line; done").await,
        "got a\ngot b\n"
    );
}

#[tokio::test]
async fn single_quotes_block_expansion_and_double_quotes_allow_it() {
    assert_eq!(stdout_of("X=v; echo '$X'").await, "$X\n");
    assert_eq!(stdout_of("X=v; echo \"$X\"").await, "v\n");
}

#[tokio::test]
async fn special_parameters_expand() {
    assert_eq!(stdout_of("false; echo $?").await, "1\n");
    assert_eq!(
        stdout_of("set_args() { echo $# $1-$2; }; set_args one two").await,
        "2 one-two\n"
    );
    assert_eq!(stdout_of("echo $0").await, "sandbox\n");
    assert_eq!(stdout_of("f() { echo \"[$3]\"; }; f a").await, "[]\n");
}

#[tokio::test]
async fn a_digit_after_a_positional_parameter_is_a_literal() {
    // `$1x` is `$1` followed by `x`, as in POSIX sh.
    assert_eq!(stdout_of("f() { echo $1x; }; f a").await, "ax\n");
    assert_eq!(stdout_of("f() { echo $1x; }; f").await, "x\n");
}

#[tokio::test]
async fn a_braced_name_that_starts_with_a_digit_is_refused() {
    // bash: `${1bad}: bad substitution`. (`${bad-name}` is valid: it defaults to `name`.)
    let err = run("echo ${1bad}").await.err().unwrap();
    assert_eq!(err.code, ErrorCode::ShellError);
}

#[tokio::test]
async fn parameter_operators_trim_and_default() {
    assert_eq!(
        stdout_of("unset y; echo ${y:-fallback}").await,
        "fallback\n"
    );
    // An unquoted empty expansion is no field, so `${y-set}` (y set but empty) vanishes.
    assert_eq!(
        stdout_of("y=; echo ${y:-fallback} ${y-set}").await,
        "fallback\n"
    );
    assert_eq!(
        stdout_of("f=archive.tar.gz; echo ${f%%.*} ${f%.gz}").await,
        "archive archive.tar\n"
    );
    assert_eq!(
        stdout_of("p=/usr/local/bin; echo ${p##*/} ${p#/usr/}").await,
        "bin local/bin\n"
    );
}

#[tokio::test]
async fn sh_dash_c_runs_its_script_in_this_shell() {
    let out = run("sh -c 'x=inside; echo $x'; echo ${x:-unset}")
        .await
        .unwrap();
    assert_eq!(out.stdout, "inside\nunset\n", "sh -c has its own scope");
    assert_eq!(out.status, 0);
}

#[tokio::test]
async fn sh_dash_c_passes_its_status_and_refuses_other_forms() {
    assert_eq!(
        run("sh -c 'exit 3'; echo after $?").await.unwrap().stdout,
        "after 3\n",
        "exit ends only the sh"
    );
    assert_eq!(run("sh -c 'false'").await.unwrap().status, 1);
    let out = run("sh script.sh").await.unwrap();
    assert_eq!(out.status, 2);
    assert!(out.stderr.contains("only -c"), "{:?}", out.stderr);
}

#[tokio::test]
async fn sh_dash_c_respects_the_step_budget_of_the_outer_run() {
    let fs = Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())));
    let config = ShellConfig {
        max_steps: 50,
        ..ShellConfig::default()
    };
    let mut shell = Shell::new(fs, config);
    let out = shell.run("sh -c 'while true; do :; done'").await;
    assert!(
        out.is_err() || out.unwrap().exit_code != 0,
        "the loop must not run forever"
    );
}

#[tokio::test]
async fn random_is_a_number_in_range_and_varies() {
    let out = run("a=$RANDOM; b=$RANDOM; c=$RANDOM; echo $a $b $c")
        .await
        .unwrap();
    let values: Vec<u32> = out
        .stdout
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(values.len(), 3);
    assert!(values.iter().all(|v| *v < 32768));
    assert!(
        !(values[0] == values[1] && values[1] == values[2]),
        "{values:?}"
    );
}

#[tokio::test]
async fn an_assigned_random_is_not_replaced() {
    assert_eq!(stdout_of("RANDOM=7; echo $RANDOM").await, "7\n");
}
