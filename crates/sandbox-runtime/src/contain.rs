//! Where `node` scripts run.
//!
//! A script can exhaust memory in ways the in-process heap limit cannot stop, and a
//! fatal V8 out-of-memory aborts the host process. So a host installs a
//! [`ScriptRunner`] that runs scripts in a separate process with an OS-level limit
//! (see `sandbox-worker`). Until a runner is installed, [`execute_script_contained`]
//! refuses to run anything: it fails closed rather than fall back to the host process.
//!
//! [`install_in_process_script_runner`] is an explicit opt-out for development and
//! tests. It gives no containment.

use crate::runtime::{execute_script_streamed_with_context, ScriptLimits, ScriptOutcome};
use crate::{ScriptContext, StreamObserver};
use async_trait::async_trait;
use sandbox_types::{ErrorCode, SandboxError};
use sandbox_vfs::Vfs;
use std::sync::{Arc, OnceLock};

/// Runs one script somewhere the host can contain it.
#[async_trait]
pub trait ScriptRunner: Send + Sync {
    /// Runs `code` under `limits`, telling `observer` each kept piece of stdout.
    /// A script failure is an `Ok` outcome. An `Err` means the runner could not
    /// run the script at all, such as when its isolation is unavailable.
    ///
    /// `context` carries the session's cwd and env into the script. Runners must not
    /// substitute host values for it: with no context, the script sees an empty env.
    async fn run(
        &self,
        code: &str,
        limits: ScriptLimits,
        observer: Option<StreamObserver>,
        vfs: Option<Arc<Vfs>>,
        context: Option<ScriptContext>,
    ) -> Result<ScriptOutcome, SandboxError>;
}

static RUNNER: OnceLock<Arc<dyn ScriptRunner>> = OnceLock::new();

/// Installs the runner that `node` uses. It can be set once per process.
pub fn install_script_runner(runner: Arc<dyn ScriptRunner>) -> Result<(), SandboxError> {
    RUNNER.set(runner).map_err(|_| {
        SandboxError::new(
            ErrorCode::Internal,
            "a script runner is already installed for this process",
        )
    })
}

/// Installs the in-process runner. This gives no containment: a script runs in the
/// host process, and a fatal out-of-memory kills it. Use it only for development and
/// tests, and never for untrusted code in a server.
pub fn install_in_process_script_runner() -> Result<(), SandboxError> {
    install_script_runner(Arc::new(InProcessRunner))
}

/// Runs a script through the installed runner. Fails closed when none is installed.
pub async fn execute_script_contained(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
) -> Result<ScriptOutcome, SandboxError> {
    execute_script_contained_with_context(code, limits, observer, vfs, None).await
}

/// Like [`execute_script_contained`], but also passes a [`ScriptContext`] (cwd,
/// session env) to the runner, so the `node:` shim reads the real session values.
pub async fn execute_script_contained_with_context(
    code: &str,
    limits: ScriptLimits,
    observer: Option<StreamObserver>,
    vfs: Option<Arc<Vfs>>,
    context: Option<ScriptContext>,
) -> Result<ScriptOutcome, SandboxError> {
    match RUNNER.get() {
        Some(runner) => runner.run(code, limits, observer, vfs, context).await,
        None => Err(SandboxError::new(
            ErrorCode::Unsupported,
            "node is disabled: no contained script runner is installed on this host",
        )),
    }
}

/// Runs scripts in the host process. See [`install_in_process_script_runner`].
pub struct InProcessRunner;

#[async_trait]
impl ScriptRunner for InProcessRunner {
    async fn run(
        &self,
        code: &str,
        limits: ScriptLimits,
        observer: Option<StreamObserver>,
        vfs: Option<Arc<Vfs>>,
        context: Option<ScriptContext>,
    ) -> Result<ScriptOutcome, SandboxError> {
        execute_script_streamed_with_context(code, limits, observer, vfs, context).await
    }
}
