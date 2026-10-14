//! The REST OpenAPI document, generated from the `utoipa` annotations in
//! `api.rs` and the `ToSchema` derives in `sandbox-types`.
//!
//! `openapi/v1/openapi.json` and `openapi/v1/openapi.yaml` at the repository
//! root are written from this document by `cargo run -p sandbox-server --bin
//! gen_openapi`. A test fails when the committed files drift from it.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Frontal Sandbox API",
        version = "v1",
        description = "REST API for sandboxes: sessions, commands, files, and snapshots. \
            Routes marked 501 are placeholders that do not do the work yet."
    ),
    paths(
        crate::api::create_session,
        crate::api::get_session,
        crate::api::destroy_session,
        crate::api::exec_command,
        crate::api::read_file,
        crate::api::write_file,
        crate::api::delete_file,
        crate::api::take_snapshot,
        crate::api::fork_snapshot,
    ),
    components(schemas(
        crate::api::ErrorBody,
        crate::api::CreateSessionResponse,
        crate::api::TakeSnapshotResponse,
        crate::api::ForkRequest,
        crate::api::ForkResponse,
        sandbox_types::SessionConfig,
        sandbox_types::SessionState,
        sandbox_types::SessionStatus,
        sandbox_types::Tier,
        sandbox_types::TierPreference,
        sandbox_types::ExecRequest,
        sandbox_types::ExecResult,
        sandbox_types::Snapshot,
        sandbox_types::ErrorCode,
        sandbox_types::policy::ResourceLimits,
        sandbox_types::policy::NetworkPolicy,
        sandbox_types::policy::NetworkPolicyRule,
        sandbox_types::policy::PolicyAction,
        sandbox_types::policy::SecretsConfig,
        sandbox_types::policy::SecretEntry,
    )),
    modifiers(&BearerAuth),
    tags(
        (name = "sessions", description = "Session lifecycle"),
        (name = "exec", description = "Running commands"),
        (name = "fs", description = "Files inside a session"),
        (name = "snapshots", description = "Snapshots and forks"),
    )
)]
pub struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("sbx_<key-id>_<secret>")
                    .build(),
            ),
        );
    }
}

/// The document. Built on each call, so it always matches the code.
pub fn document() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// Pretty JSON, with a trailing newline, as committed to `openapi/v1/`.
pub fn json() -> Result<String, serde_json::Error> {
    let mut text = document().to_pretty_json()?;
    text.push('\n');
    Ok(text)
}

/// YAML, as committed to `openapi/v1/`.
pub fn yaml() -> Result<String, Box<dyn std::error::Error>> {
    Ok(document().to_yaml()?)
}
