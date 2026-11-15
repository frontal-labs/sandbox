//! Integration tests for the V8 runtime host.

use sandbox_runtime::{execute_script, ScriptLimits, ScriptOutcome};
use std::time::Duration;

async fn run(code: &str) -> ScriptOutcome {
    execute_script(code, ScriptLimits::default()).await.unwrap()
}

#[tokio::test]
async fn console_log_is_captured() {
    let out = run(r#"console.log("hello", 1 + 2, {a: 1});"#).await;
    assert_eq!(out.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello 3 {\"a\":1}\n");
}

#[tokio::test]
async fn thrown_errors_are_script_failures_not_host_errors() {
    let out = run("throw new Error('boom');").await;
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("boom"));
}

#[tokio::test]
async fn infinite_loops_are_terminated_by_the_watchdog() {
    let limits = ScriptLimits {
        timeout: Duration::from_millis(200),
        ..ScriptLimits::default()
    };
    let out = execute_script("while (true) {}", limits).await.unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
}

#[tokio::test]
async fn output_is_capped() {
    let limits = ScriptLimits {
        max_output_bytes: 10,
        ..ScriptLimits::default()
    };
    let out = execute_script(
        "for (let i = 0; i < 100; i++) console.log('abcdef');",
        limits,
    )
    .await
    .unwrap();
    assert_eq!(out.stdout.len(), 10);
    assert!(String::from_utf8_lossy(&out.stderr).contains("truncated"));
}

#[tokio::test]
async fn each_run_starts_with_fresh_state() {
    run("globalThis.leak = 42;").await;
    let out = run("console.log(typeof globalThis.leak);").await;
    assert_eq!(String::from_utf8_lossy(&out.stdout), "undefined\n");
}

#[tokio::test]
async fn host_filesystem_and_environment_are_not_reachable() {
    // Deno is deleted. `require` is the only node: shim global; `process` is
    // reachable through `require("process")`, not as a global.
    let out = run("console.log(typeof Deno, typeof process, typeof require);").await;
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "undefined undefined function\n"
    );
}

#[tokio::test]
async fn require_resolves_node_modules() {
    let out = run(r#"
        const fs = require("node:fs");
        const path = require("path");
        console.log(typeof fs.readFileSync, typeof path.join);
        "#)
    .await;
    assert_eq!(out.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "function function\n");
}

#[tokio::test]
async fn require_unknown_module_throws() {
    let out = run("require('node:nonexistent')").await;
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot find module"));
}

#[tokio::test]
async fn process_env_starts_empty_without_a_session() {
    // Without a ScriptContext the host environment must not be visible.
    let out = run(r#"
        const env = require("process").env;
        const before = Object.keys(env).length;
        env.FOO = "bar";
        console.log(before, env.FOO, Object.keys(env).length);
        "#)
    .await;
    assert_eq!(out.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0 bar 1\n");
}

#[tokio::test]
async fn default_deno_ops_are_unreachable_from_scripts() {
    // Attempts to reach the default ops (including op_print, which writes to
    // host stdout) must fail. Nothing should be written to the host stream.
    let out = run("try { globalThis.Deno.core.print('forged\\n', false); } catch (e) { console.log('blocked'); }").await;
    assert_eq!(out.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "blocked\n");
}

#[tokio::test]
async fn user_scripts_can_reuse_names_declared_by_the_shim() {
    // The bootstrap's top-level bindings must not collide with a script's own declarations.
    let out = run(r#"
        class AssertionError extends Error {}
        const timeOps = 1;
        console.log(new AssertionError("x") instanceof Error, timeOps);
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "true 1\n");
}

#[tokio::test]
async fn node_shim_modules_behave_like_node() {
    let out = run(r#"
        const { StringDecoder } = require("string_decoder");
        const dec = new StringDecoder("utf8");
        const euro = [0xe2, 0x82, 0xac]; // "€" in UTF-8
        const first = dec.write(euro.slice(0, 1));
        const rest = dec.write(euro.slice(1));
        const qs = require("querystring").parse("a=%E0&b=2");
        const assert = require("assert");
        assert({ x: [1, { y: 2 }] }.x.length === 2, "callable");
        assert.strict.deepEqual({ a: [1, 2], b: { c: 3 } }, { a: [1, 2], b: { c: 3 } });
        let threw = false;
        try { require("readline").createInterface({}); } catch (e) { threw = true; }
        console.log(JSON.stringify([first, rest, qs.a, qs.b, threw]));
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "[\"\",\"€\",\"%E0\",\"2\",true]\n"
    );
}

#[tokio::test]
async fn heap_growth_is_stopped_before_it_aborts_the_process() {
    let limits = ScriptLimits {
        max_heap_bytes: 32 * 1024 * 1024,
        timeout: Duration::from_secs(20),
        ..ScriptLimits::default()
    };
    let out = execute_script(
        "const a = []; while (true) a.push({ x: 'y'.repeat(1000) });",
        limits,
    )
    .await
    .unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("heap limit"));
}

#[tokio::test]
async fn create_hash_produces_correct_digests() {
    let out = run(r#"
        const crypto = require("crypto");
        const sha256 = crypto.createHash("sha256");
        sha256.update("hello");
        console.log(sha256.digest("hex"));
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\n"
    );
}

#[tokio::test]
async fn create_hash_supports_sha1() {
    let out = run(r#"
        const crypto = require("crypto");
        console.log(crypto.createHash("sha1").update("hello").digest("hex"));
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // sha1("hello") = aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d\n"
    );
}

#[tokio::test]
async fn create_hash_supports_sha512() {
    let out = run(r#"
        const crypto = require("crypto");
        console.log(crypto.createHash("sha512").update("hello").digest("hex"));
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // sha512("hello") = 9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043\n"
    );
}

#[tokio::test]
async fn create_hash_accumulates_across_updates() {
    let out = run(r#"
        const crypto = require("crypto");
        const h = crypto.createHash("sha256");
        h.update("hel");
        h.update("lo");
        console.log(h.digest("hex"));
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // sha256("hello") — same as single update
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\n"
    );
}

#[tokio::test]
async fn create_hash_default_encoding_is_buffer() {
    let out = run(r#"
        const crypto = require("crypto");
        const digest = crypto.createHash("sha256").update("hello").digest();
        console.log(digest.length, digest instanceof Uint8Array);
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "32 true\n");
}

#[tokio::test]
async fn create_hash_rejects_unsupported_algorithm() {
    let out = run(r#"
        const crypto = require("crypto");
        try { crypto.createHash("md5"); } catch (e) { console.log(e.message); }
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("unsupported algorithm"));
}

#[tokio::test]
async fn create_hash_accepts_dash_and_underscore_variants() {
    let out = run(r#"
        const crypto = require("crypto");
        const a = crypto.createHash("sha-256").update("x").digest("hex");
        const b = crypto.createHash("sha_256").update("x").digest("hex");
        console.log(a === b);
        "#)
    .await;
    assert_eq!(
        out.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "true\n");
}
