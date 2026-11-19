//! Escape tests for the V8 isolate tier (SECURITY_MODEL.md, "Required before any public launch").
//!
//! Hostile JavaScript runs in the real runtime, and the test checks, from outside, that it could
//! not reach the host filesystem, environment, identity, or network, that it could not recover
//! the deleted `Deno` global or a raw op, and that the CPU limits hold against inputs that try to
//! evade the watchdog.
//!
//! These run the in-process path (`execute_script_streamed`). The memory-exhaustion probes that
//! can abort a process, and the OS-level layers, are in `cli/tests/contained_escape.rs`, which
//! needs Linux with delegated cgroups.
//!
//! **Convention.** A passing test states a property the code holds. A test marked
//! `#[ignore = "KNOWN GAP: ..."]` states the property the code should hold and does not;
//! `cargo test -- --ignored` lists every open gap. See docs/ESCAPE_TESTS.md.

use sandbox_cas::MemStore;
use sandbox_runtime::{execute_script_streamed, ScriptLimits, ScriptOutcome};
use sandbox_types::hash::ContentHash;
use sandbox_vfs::{OpenFlags, SessionFs, Vfs};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CANARY: &str = "HOST-V8-CANARY-5c0e7a13";

fn limits(timeout_ms: u64) -> ScriptLimits {
    ScriptLimits {
        timeout: Duration::from_millis(timeout_ms),
        ..ScriptLimits::default()
    }
}

/// Runs `code` with a hard outer deadline, so a watchdog that fails to fire fails the test
/// instead of hanging the suite.
async fn run_with(code: &str, limits: ScriptLimits, vfs: Option<Arc<Vfs>>) -> ScriptOutcome {
    tokio::time::timeout(
        Duration::from_secs(40),
        execute_script_streamed(code, limits, None, vfs),
    )
    .await
    .unwrap_or_else(|_| panic!("the script was not stopped within 40s: {code}"))
    .expect("the runtime ran the script")
}

async fn run(code: &str) -> ScriptOutcome {
    run_with(code, ScriptLimits::default(), None).await
}

fn stdout(out: &ScriptOutcome) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &ScriptOutcome) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn new_vfs() -> Arc<Vfs> {
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
    let handle = vfs.open(Path::new(path), flags).await.unwrap();
    vfs.write(&handle, 0, data).await.unwrap();
    vfs.close(&handle);
}

struct HostCanary {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
}

impl HostCanary {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("escape-v8-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("canary.txt");
        std::fs::write(&file, CANARY).unwrap();
        Self { dir, file }
    }

    fn path(&self) -> &str {
        self.file.to_str().unwrap()
    }

    fn assert_untouched(&self) {
        let names: Vec<_> = std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["canary.txt".to_string()],
            "host directory changed"
        );
        assert_eq!(std::fs::read_to_string(&self.file).unwrap(), CANARY);
    }
}

impl Drop for HostCanary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The text of a JS string literal for `s`, so a host path can be embedded in a script.
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

// ==========================================================================================
// Reachable surface: globals, ops, and the deleted `Deno`
// ==========================================================================================

#[tokio::test]
async fn deno_and_host_globals_cannot_be_recovered() {
    let out = run(r#"
        const r = {};
        r.typeofDeno = typeof Deno;
        r.viaSloppyThis = (function () { return typeof this === 'undefined' ? 'none' : typeof this.Deno; })();
        r.viaFunctionCtor = typeof (new Function('return this')()).Deno;
        r.viaIndirectEval = typeof (0, eval)('this').Deno;
        r.viaGlobalThisProto = typeof Object.getPrototypeOf(globalThis).Deno;
        r.viaProp = String(Object.getOwnPropertyNames(globalThis).includes('Deno'));
        r.viaDescriptor = String(Object.getOwnPropertyDescriptor(globalThis, 'Deno'));
        r.viaAsyncFn = typeof (async function () {}).constructor('return this')().Deno;
        r.viaGenerator = typeof (function* () {}).constructor('return this')().Deno;
        r.hostGlobals = ['process', 'Bun', 'fetch', 'XMLHttpRequest', 'WebSocket', 'Worker',
          'SharedWorker', 'EventSource', 'WebTransport', 'navigator', 'location', '__dirname',
          '__filename', 'module', 'exports', 'Buffer', 'global', 'window', 'self']
          .filter((n) => typeof globalThis[n] !== 'undefined').join(',');
        r.suspicious = Object.getOwnPropertyNames(globalThis)
          .filter((n) => /deno|core|ops?$|bootstrap|internal|^__/i.test(n)).join(',');
        for (const k of Object.keys(r)) console.log(k + '=' + r[k]);
    "#)
    .await;
    assert_eq!(out.exit_code, 0, "{}", stderr(&out));
    let text = stdout(&out);
    eprintln!("{text}");
    for key in [
        "typeofDeno",
        "viaSloppyThis",
        "viaFunctionCtor",
        "viaIndirectEval",
        "viaGlobalThisProto",
        "viaAsyncFn",
        "viaGenerator",
    ] {
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("{key}=")))
            .unwrap();
        assert!(
            line.ends_with("=undefined") || line.ends_with("=none"),
            "Deno was recovered: {line}"
        );
    }
    assert!(text.contains("viaProp=false"), "{text}");
    assert!(text.contains("viaDescriptor=undefined"), "{text}");
    // `suspicious=__bootstrap` is reported, not asserted here: see
    // `bootstrap_internals_are_not_reachable`, which is a known gap.
}

#[tokio::test]
#[ignore = "KNOWN GAP: `__bootstrap.core` leaves the whole deno_core op table reachable, so `delete globalThis.Deno` does not remove raw ops"]
async fn no_raw_host_op_is_reachable_beyond_the_known_set() {
    // Walk every object reachable from `globalThis` (own properties, accessors, prototypes) and
    // collect functions named `op_*`. Those are raw host capabilities. Adding an op is a security
    // decision (ops.rs), so a new name here must be reviewed and added on purpose.
    let out = run(r#"
        const seen = new Set(), found = new Set(), queue = [globalThis];
        let steps = 0;
        while (queue.length && steps++ < 400000) {
          const o = queue.pop();
          if (o === null || (typeof o !== 'object' && typeof o !== 'function') || seen.has(o)) continue;
          seen.add(o);
          if (typeof o === 'function' && typeof o.name === 'string' && /^op_/.test(o.name)) found.add(o.name);
          let keys = [];
          try { keys = Reflect.ownKeys(o); } catch (e) {}
          for (const k of keys) {
            let d; try { d = Object.getOwnPropertyDescriptor(o, k); } catch (e) { continue; }
            if (!d) continue;
            if ('value' in d) queue.push(d.value); else { if (d.get) queue.push(d.get); if (d.set) queue.push(d.set); }
          }
          try { queue.push(Object.getPrototypeOf(o)); } catch (e) {}
        }
        console.log('OPS=' + [...found].sort().join(','));
        console.log('OBJECTS=' + seen.size);
    "#)
    .await;
    assert_eq!(out.exit_code, 0, "{}", stderr(&out));
    let text = stdout(&out);
    eprintln!("{text}");
    let ops = text
        .lines()
        .find(|l| l.starts_with("OPS="))
        .unwrap()
        .trim_start_matches("OPS=");
    let allowed = [
        "op_sandbox_print",
        "op_sandbox_env",
        "op_sandbox_cwd",
        "op_sandbox_now",
        "op_sandbox_hash",
        "op_stdout_write",
        "op_stderr_write",
        "op_fs_read",
        "op_fs_write",
        "op_fs_stat",
        "op_fs_readdir",
        "op_fs_mkdir",
        "op_fs_rm",
        "op_fs_rename",
        "op_fs_symlink",
        "op_fs_readlink",
        "op_fs_truncate",
    ];
    for op in ops.split(',').filter(|s| !s.is_empty()) {
        assert!(
            allowed.contains(&op),
            "unreviewed raw op reachable from the guest: {op}"
        );
    }
}

#[tokio::test]
async fn dynamic_import_cannot_load_host_files_urls_or_modules() {
    // Weak while Promise jobs never run (`async_code_and_timers_run`): the import's result cannot be
    // observed, so this proves only that no host data reaches stdout or stderr. The static
    // evidence is that `RuntimeOptions` sets no module loader, so deno_core's noop loader applies.
    let canary = HostCanary::new("import");
    let code = format!(
        r#"
        (async () => {{
          for (const spec of [
            'file://' + {path}, {path}, 'file:///etc/passwd', 'node:fs', 'fs', 'data:text/javascript,export default 1',
            'https://example.com/x.js', 'http://127.0.0.1:1/x.js', './x.js', '../../etc/passwd', 'ext:core/mod.js',
            'ext:deno_web/00_infra.js', 'internal:core', 'deno:core', 'npm:left-pad', 'jsr:@std/path',
          ]) {{
            try {{ const m = await import(spec); console.log('LOADED ' + spec + ' keys=' + Object.keys(m)); }}
            catch (e) {{ console.log('REFUSED ' + spec); }}
          }}
        }})();
        "#,
        path = js(canary.path())
    );
    let out = run(&code).await;
    let text = stdout(&out);
    eprintln!("{text}\n{}", stderr(&out));
    assert!(
        !text.contains("LOADED"),
        "a dynamic import succeeded:\n{text}"
    );
    assert!(!text.contains(CANARY) && !stderr(&out).contains(CANARY));
    canary.assert_untouched();
}

#[tokio::test]
async fn the_node_shim_reveals_nothing_about_the_host() {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let pid = std::process::id();
    let hostname = std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    std::env::set_var("ESCAPE_V8_ENV_CANARY", CANARY);

    let out = run(r#"
        const out = {};
        const safe = new Set(['cwd', 'uptime', 'hrtime', 'memoryUsage', 'cpuUsage', 'umask', 'getuid',
          'geteuid', 'getgid', 'getegid', 'resourceUsage', 'emitWarning']);
        const os = require('os');
        for (const k of Object.keys(os)) {
          try { out['os.' + k] = typeof os[k] === 'function' ? JSON.stringify(os[k]()) : JSON.stringify(os[k]); }
          catch (e) { out['os.' + k] = 'throws'; }
        }
        const p = require('process');
        for (const k of Object.keys(p)) {
          try {
            if (typeof p[k] === 'function') { if (safe.has(k)) out['process.' + k] = JSON.stringify(p[k]()); }
            else out['process.' + k] = JSON.stringify(p[k]);
          } catch (e) { out['process.' + k] = 'throws'; }
        }
        out['env'] = JSON.stringify(require('process').env);
        out['stack'] = String(new Error('x').stack);
        console.log(JSON.stringify(out));
    "#)
    .await;
    assert_eq!(out.exit_code, 0, "{}", stderr(&out));
    let blob = stdout(&out);
    eprintln!("{blob}");
    assert!(
        !blob.contains(CANARY),
        "the host environment reached the guest"
    );
    for (label, value) in [
        ("username", user.as_str()),
        ("home directory", home.as_str()),
        ("working directory", cwd.as_str()),
        ("executable path", exe.as_str()),
        ("hostname", hostname.as_str()),
    ] {
        if value.len() >= 4 && value != "/" {
            assert!(
                !blob.contains(value),
                "the {label} {value:?} reached the guest"
            );
        }
    }
    assert!(
        !blob.contains(&format!("\"process.pid\":\"{pid}\""))
            && !blob.contains(&format!("\"process.ppid\":\"{pid}\"")),
        "the host pid reached the guest"
    );
}

// ==========================================================================================
// Filesystem: the VFS is the only filesystem
// ==========================================================================================

#[test]
fn the_fs_shim_never_returns_host_data() {
    // Each attempt runs in a child process with a VFS installed. Today the fs shim aborts the
    // process on its first call (see `the_fs_shim_returns_file_contents`), so this is a weak
    // check: it proves no host byte arrives, and it will start to mean more once the shim works.
    let canary = HostCanary::new("fs");
    let host = canary.path().to_string();
    let host_dir = canary.dir.to_str().unwrap().to_string();
    let climbing = format!("{}{}", "../".repeat(40), host.trim_start_matches('/'));
    for expr in [
        format!("fs.readFileSync({})", js(&host)),
        format!("fs.readFileSync({})", js(&climbing)),
        format!("fs.readdirSync({})", js(&host_dir)),
        "fs.readFileSync('/etc/passwd')".to_string(),
        "fs.readFileSync('/proc/self/environ')".to_string(),
        format!(
            "fs.writeFileSync({}, 'pwn')",
            js(&format!("{host_dir}/pwn.txt"))
        ),
        format!("fs.rmSync({})", js(&host)),
        format!("fs.renameSync({}, '/moved')", js(&host)),
        format!(
            "fs.symlinkSync({}, '/link'); fs.readFileSync('/link')",
            js(&host)
        ),
    ] {
        let code = format!("const fs = require('fs'); console.log(String({expr}));");
        let end = child_probe("script_vfs", Some(&code));
        assert!(
            !end.stdout.contains(CANARY) && !end.stderr.contains(CANARY),
            "a host file leaked through `{expr}`"
        );
    }
    canary.assert_untouched();
}

#[test]
#[ignore = "KNOWN GAP: the fs shim calls async ops from sync functions; with a VFS installed the first call aborts the process (deno_unsync::spawn outside a LocalSet)"]
fn the_fs_shim_returns_file_contents() {
    let end = child_probe(
        "script_vfs",
        Some("const fs = require('fs'); console.log(String.fromCharCode(...fs.readFileSync('/inside.txt')));"),
    );
    assert!(end.success, "the process died (signal {:?})", end.signal);
    assert!(
        end.stdout.contains("stdout=\"inside\\n\""),
        "{}",
        end.stdout
    );
}

#[tokio::test]
#[ignore = "KNOWN GAP: an fs op with no VFS installed panics inside a V8 callback, which aborts the process (the contained child always has no VFS)"]
async fn a_script_with_no_vfs_cannot_reach_a_filesystem_at_all() {
    let canary = HostCanary::new("novfs");
    let code = format!(
        "const fs = require('fs'); try {{ console.log(fs.readFileSync({}, 'utf8')); }} catch (e) {{ console.log('ERR'); }}",
        js(canary.path())
    );
    // Run in a child process: an op that finds no VFS is a panic, and that must not abort the host.
    let end = child_probe("fs_without_vfs", Some(&code));
    eprintln!("{end:?}");
    assert!(!end.stdout.contains(CANARY) && !end.stderr.contains(CANARY));
    assert!(
        end.success,
        "the host process died when a script used fs with no VFS: signal {:?}, stderr tail {:?}",
        end.signal,
        end.stderr
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    canary.assert_untouched();
}

// ==========================================================================================
// Network and processes
// ==========================================================================================

#[tokio::test]
async fn there_is_no_network_api_and_nothing_connects() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let code = format!(
        r#"
        const results = [];
        const attempt = (label, f) => {{
          try {{ const v = f(); results.push(label + ' REACHED ' + typeof v); }}
          catch (e) {{ results.push(label + ' refused'); }}
        }};
        attempt('fetch', () => fetch('http://127.0.0.1:{port}/'));
        attempt('xhr', () => {{ const x = new XMLHttpRequest(); x.open('GET', 'http://127.0.0.1:{port}/'); x.send(); }});
        attempt('websocket', () => new WebSocket('ws://127.0.0.1:{port}/'));
        attempt('eventsource', () => new EventSource('http://127.0.0.1:{port}/'));
        for (const m of ['http', 'https', 'http2', 'net', 'dgram', 'tls', 'dns', 'cluster', 'inspector', 'vm', 'v8', 'worker_threads', 'repl', 'readline']) {{
          attempt('require ' + m, () => {{ const mod = require(m); if (m === 'http' || m === 'https') mod.get('http://127.0.0.1:{port}/'); if (m === 'net') mod.connect({port}, '127.0.0.1'); if (m === 'dgram') mod.createSocket('udp4').send('x', {port}, '127.0.0.1'); if (m === 'dns') mod.lookup('localhost', () => {{}}); if (m === 'worker_threads') new mod.Worker('x'); if (m === 'vm') mod.runInThisContext('1'); return mod; }});
        }}
        attempt('child_process.exec', () => require('child_process').exec('id'));
        attempt('child_process.spawn', () => require('child_process').spawn('id'));
        attempt('child_process.execSync', () => require('child_process').execSync('id'));
        attempt('process.binding', () => require('process').binding('fs'));
        attempt('process.dlopen', () => require('process').dlopen({{}}, '/lib/x.so'));
        attempt('process.kill', () => require('process').kill(process_pid(), 0));
        function process_pid() {{ return require('process').pid; }}
        console.log(results.join('\n'));
        "#
    );
    let out = run(&code).await;
    let text = stdout(&out);
    eprintln!("{text}\n{}", stderr(&out));
    tokio::time::sleep(Duration::from_millis(200)).await;
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("the guest opened a connection to the host: {other:?}"),
    }
    for danger in [
        "fetch REACHED",
        "xhr REACHED",
        "websocket REACHED",
        "eventsource REACHED",
        "child_process.exec REACHED",
        "child_process.spawn REACHED",
        "child_process.execSync REACHED",
        "require net REACHED",
        "require http REACHED",
        "require dgram REACHED",
    ] {
        assert!(!text.contains(danger), "{danger}\n{text}");
    }
}

// ==========================================================================================
// CPU and memory
// ==========================================================================================

async fn assert_stopped_by_watchdog(label: &str, code: &str) {
    let started = Instant::now();
    let out = run_with(code, limits(400), None).await;
    let elapsed = started.elapsed();
    assert_eq!(
        out.exit_code,
        1,
        "{label}: exit {}: {}",
        out.exit_code,
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("timed out") || stderr(&out).contains("terminated"),
        "{label}: {}",
        stderr(&out)
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "{label}: stopped after {elapsed:?}"
    );
}

#[tokio::test]
async fn the_watchdog_stops_every_known_way_to_spin_or_block() {
    for (label, code) in [
        ("busy loop", "while (true) {}"),
        ("empty for", "for (;;) {}"),
        ("Atomics.wait forever", "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0);"),
        ("regex backtracking", "/^(a+)+$/.test('a'.repeat(60) + 'b');"),
        ("regex alternation", "/(x+x+)+y/.test('x'.repeat(60));"),
        ("sort with slow comparator", "const a = Array.from({length: 1e6}, (_, i) => i); a.sort(() => { for (let i = 0; i < 1e4; i++); return 0; });"),
        ("JSON.stringify huge", "let o = {}; for (let i = 0; i < 1e3; i++) o = {a: o, b: o}; JSON.stringify(o);"),
        ("string concat loop", "let s = ''; while (true) { s += 'x'.repeat(1000); if (s.length > 1e8) s = ''; }"),
        ("try/finally loop", "while (true) { try { throw 1; } catch (e) {} finally { continue; } }"),
        ("catch-all terminate", "while (true) { try { while (true) {} } catch (e) {} }"),
        ("generator loop", "function* g() { while (true) yield 1; } for (const x of g()) {}"),
        ("Proxy loop", "const p = new Proxy({}, { get() { while (true) {} } }); p.x;"),
        ("toString trap", "String({ toString() { while (true) {} } });"),
        ("getter in JSON", "JSON.stringify({ get a() { while (true) {} } });"),
    ] {
        assert_stopped_by_watchdog(label, code).await;
    }
}

#[tokio::test]
async fn expensive_bigint_work_fails_fast_or_is_stopped() {
    // V8 refuses a result past its BigInt size cap at once. Either way the run ends, in error.
    let started = Instant::now();
    let out = run_with("2n ** (2n ** 40n);", limits(400), None).await;
    assert_eq!(out.exit_code, 1, "{}", stderr(&out));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn a_script_cannot_catch_the_termination_and_carry_on() {
    // After the watchdog fires, no further guest code may run, including `finally` blocks.
    let out = run_with(
        "try { while (true) {} } finally { console.log('RAN AFTER TERMINATION'); }",
        limits(300),
        None,
    )
    .await;
    assert_eq!(out.exit_code, 1);
    assert!(
        !stdout(&out).contains("RAN AFTER TERMINATION"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn deep_recursion_and_deep_data_are_script_errors() {
    // Each script runs in a child process, because a stack overflow that V8 fails to catch
    // would abort the process.
    let mut failures = Vec::new();
    for (label, code) in [
        ("recursion", "function f() { f(); } f();"),
        (
            "mutual recursion",
            "function a() { b(); } function b() { a(); } a();",
        ),
        (
            "deep JSON stringify",
            "let o = []; for (let i = 0; i < 1e6; i++) o = [o]; JSON.stringify(o);",
        ),
        (
            "deep array flat",
            "let o = []; for (let i = 0; i < 1e6; i++) o = [o]; o.flat(Infinity);",
        ),
        (
            "deep toString",
            "let o = []; for (let i = 0; i < 1e6; i++) o = [o]; String(o);",
        ),
        (
            "deep regex",
            "new RegExp('('.repeat(1e5) + ')'.repeat(1e5));",
        ),
        (
            "deep parse",
            "eval('('.repeat(1e6) + '1' + ')'.repeat(1e6));",
        ),
        (
            "deep nested function",
            "eval('(function(){'.repeat(1e5) + '}) '.repeat(1e5));",
        ),
        (
            "Function ctor huge",
            "new Function('return ' + '1+'.repeat(1e6) + '1');",
        ),
    ] {
        let end = child_probe("script", Some(code));
        if !end.success || !end.stdout.contains("CHILD-RESULT") {
            failures.push(format!("{label}: signal {:?}", end.signal));
        }
    }
    assert!(
        failures.is_empty(),
        "the process died:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "KNOWN GAP: a 10 MB nested-array JSON.parse reaches V8's fatal out-of-memory in-process, despite the near-heap-limit callback (documented for execute_script)"]
fn a_deeply_nested_json_parse_cannot_abort_the_process() {
    let end = child_probe(
        "script",
        Some("JSON.parse('['.repeat(5e6) + ']'.repeat(5e6));"),
    );
    assert!(
        end.success && end.stdout.contains("CHILD-RESULT"),
        "the process died (signal {:?}): {}",
        end.signal,
        end.stderr
            .lines()
            .find(|l| l.contains("Fatal"))
            .unwrap_or("")
    );
}

#[tokio::test]
async fn output_flood_is_capped_whatever_the_channel() {
    let cap = 4096usize;
    let l = ScriptLimits {
        max_output_bytes: cap,
        timeout: Duration::from_secs(3),
        ..ScriptLimits::default()
    };
    for (label, code) in [
        (
            "console.log loop",
            "for (;;) console.log('x'.repeat(1000));",
        ),
        (
            "one huge write",
            "console.log('x'.repeat(50 * 1024 * 1024));",
        ),
        (
            "process.stdout.write",
            "for (;;) process.stdout.write('x'.repeat(1000));",
        ),
        (
            "process.stderr.write",
            "for (;;) process.stderr.write('x'.repeat(1000));",
        ),
        (
            "console.error loop",
            "for (;;) console.error('x'.repeat(1000));",
        ),
    ] {
        let out = run_with(code, l, None).await;
        assert!(
            out.stdout.len() <= cap,
            "{label}: stdout {} bytes",
            out.stdout.len()
        );
        assert!(
            out.stderr.len() <= cap + 4096,
            "{label}: stderr {} bytes",
            out.stderr.len()
        );
    }
}

#[tokio::test]
#[ignore = "KNOWN GAP: V8's WebAssembly memory is allocated outside the heap limit"]
async fn webassembly_cannot_take_memory_outside_the_heap_cap() {
    // Eight 1 GiB memories. Pages are not touched, so little memory becomes resident, but the
    // reservation shows that the heap limit does not count them.
    let out = run_with(
        r#"
        let made = 0;
        const keep = [];
        try { for (let i = 0; i < 8; i++) { keep.push(new WebAssembly.Memory({ initial: 16384 })); made++; } }
        catch (e) {}
        console.log(typeof WebAssembly + ' ' + made);
        "#,
        ScriptLimits::default(),
        None,
    )
    .await;
    let text = stdout(&out);
    eprintln!("{text} {}", stderr(&out));
    let made: u32 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert_eq!(
        made, 0,
        "WebAssembly.Memory reserved {made} GiB under a 64 MiB heap cap"
    );
}

// ==========================================================================================
// Isolation between runs
// ==========================================================================================

#[tokio::test]
async fn prototype_pollution_in_one_run_is_invisible_to_the_next() {
    // Every built-in a script can mutate. If an isolate were reused, the next run would see this.
    let pollute = r#"
        Object.prototype.polluted = 'yes';
        Array.prototype.polluted = 'yes';
        Function.prototype.polluted = 'yes';
        String.prototype.polluted = 'yes';
        Promise.prototype.polluted = 'yes';
        globalThis.leaked = 'yes';
        globalThis.require = () => 'hijacked';
        Array.prototype.push = function () { throw new Error('push hijacked'); };
        JSON.stringify = () => 'hijacked';
        Symbol.for('leak');
        Math.random = () => 4;
        Date.now = () => 0;
        try { Object.defineProperty(globalThis, 'console', { value: { log: () => {} } }); } catch (e) {}
    "#;
    let observe = r#"
        console.log([
          typeof ({}).polluted, typeof [].polluted, typeof (function () {}).polluted,
          typeof ''.polluted, typeof leaked, typeof require('path').join,
          [1].concat([2]).length, JSON.stringify({ a: 1 }), Math.random() === 4, Date.now() === 0,
        ].join(' '));
    "#;
    for _ in 0..12 {
        let polluted = run(pollute).await;
        assert!(polluted.exit_code == 0 || polluted.exit_code == 1);
        let seen = run(observe).await;
        assert_eq!(
            stdout(&seen).trim(),
            "undefined undefined undefined undefined undefined function 2 {\"a\":1} false false",
            "state crossed runs: {} {}",
            stdout(&seen),
            stderr(&seen)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_runs_do_not_see_each_others_state() {
    let mut tasks = Vec::new();
    for i in 0..16u32 {
        tasks.push(tokio::spawn(async move {
            let code = format!(
                "globalThis.secret = 'tenant-{i}'; let n = 0; for (let k = 0; k < 200000; k++) n += k; \
                 Array.prototype.tenant = 'tenant-{i}'; console.log(globalThis.secret + ' ' + [].tenant);"
            );
            let out = run(&code).await;
            (i, stdout(&out))
        }));
    }
    for task in tasks {
        let (i, text) = task.await.unwrap();
        assert_eq!(
            text.trim(),
            format!("tenant-{i} tenant-{i}"),
            "run {i} saw another run's state"
        );
    }
}

#[tokio::test]
#[ignore = "KNOWN GAP: `__bootstrap` (primordials, core, internals) stays on globalThis"]
async fn bootstrap_internals_are_not_reachable() {
    let out = run("console.log(typeof __bootstrap, typeof globalThis.__bootstrap);").await;
    assert_eq!(
        stdout(&out).trim(),
        "undefined undefined",
        "{}",
        stdout(&out)
    );
}

#[test]
#[ignore = "KNOWN GAP: `__bootstrap.core.ops.op_print` writes straight to the host process's stdout and stderr, past the output cap and, in a contained child, into the host's frame channel"]
fn a_guest_cannot_write_to_the_host_process_output() {
    let end = child_probe(
        "script",
        Some("__bootstrap.core.ops.op_print('RAW-HOST-STDOUT-MARKER\\n', false); __bootstrap.core.ops.op_print('RAW-HOST-STDERR-MARKER\\n', true);"),
    );
    eprintln!("{end:?}");
    assert!(
        !end.stdout.contains("RAW-HOST-STDOUT-MARKER")
            && !end.stderr.contains("RAW-HOST-STDERR-MARKER"),
        "the guest wrote to the host process's own stdout/stderr"
    );
}

#[test]
#[ignore = "KNOWN GAP: Promise jobs are never run and `setTimeout` is not defined, so async/await, .then and timers silently do nothing"]
fn async_code_and_timers_run() {
    let end = child_probe(
        "script",
        Some("(async () => { await null; console.log('after await'); })(); Promise.resolve().then(() => console.log('after then')); console.log(typeof setTimeout);"),
    );
    assert!(end.success, "signal {:?}", end.signal);
    assert!(
        end.stdout.contains("after await")
            && end.stdout.contains("after then")
            && end.stdout.contains("function"),
        "{}",
        end.stdout
    );
}

// --- Child-process probes ---------------------------------------------------------------------

#[derive(Debug)]
struct ChildEnd {
    success: bool,
    signal: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs `probe` in a child copy of this test binary, so an abort does not end the suite.
fn child_probe(probe: &str, code: Option<&str>) -> ChildEnd {
    use std::os::unix::process::ExitStatusExt;
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child_entry", "--nocapture", "--test-threads=1"])
        .env("ESCAPE_V8_CHILD", probe);
    if let Some(code) = code {
        cmd.env("ESCAPE_V8_CODE", code);
    }
    let out = cmd.output().expect("the child probe starts");
    ChildEnd {
        success: out.status.success(),
        signal: out.status.signal(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The entry point of the child probes. It does nothing in a normal run.
#[test]
fn child_entry() {
    let Ok(probe) = std::env::var("ESCAPE_V8_CHILD") else {
        return;
    };
    let code = std::env::var("ESCAPE_V8_CODE").unwrap_or_default();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        match probe.as_str() {
            // The contained child runs scripts with no VFS (`script.rs` passes `None`).
            "fs_without_vfs" => {
                let out = execute_script_streamed(&code, ScriptLimits::default(), None, None).await;
                match out {
                    Ok(o) => println!(
                        "CHILD-RESULT exit={} stdout={:?} stderr={:?}",
                        o.exit_code,
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                    ),
                    Err(e) => println!("CHILD-RESULT error={}", e.message),
                }
            }
            // Any script, with an explicit VFS flag, so an abort is observed and not fatal.
            "script" | "script_vfs" => {
                let vfs = (probe == "script_vfs").then(|| {
                    let vfs = new_vfs();
                    let handle = futures_lite_block_on(async {
                        put(&vfs, "/inside.txt", b"inside").await;
                    });
                    let _ = handle;
                    vfs
                });
                let timeout = std::env::var("ESCAPE_V8_TIMEOUT_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5_000);
                let out = execute_script_streamed(&code, limits(timeout), None, vfs.clone()).await;
                match out {
                    Ok(o) => println!(
                        "CHILD-RESULT exit={} stdout={:?} stderr={:?}",
                        o.exit_code,
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                            .chars()
                            .take(300)
                            .collect::<String>()
                    ),
                    Err(e) => println!("CHILD-RESULT error={}", e.message),
                }
            }
            other => panic!("unknown child probe {other}"),
        }
    });
}

fn futures_lite_block_on<F: std::future::Future>(f: F) -> F::Output {
    // A tiny executor for fixture setup inside the child (the runtime is already running there).
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}
