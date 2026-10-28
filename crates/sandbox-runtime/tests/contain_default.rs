//! Without an installed runner, `node` must refuse to run rather than run in the host.
//! This binary installs nothing, so the default is what it sees.

use sandbox_runtime::{execute_script_contained, ScriptLimits};
use sandbox_types::ErrorCode;

#[tokio::test]
async fn node_is_refused_when_no_contained_runner_is_installed() {
    let err = execute_script_contained("console.log(1)", ScriptLimits::default(), None)
        .await
        .expect_err("node must not run without a contained runner");
    assert_eq!(err.code, ErrorCode::Unsupported);
}
