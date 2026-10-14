//! Writes `openapi/v1/openapi.json` and `openapi/v1/openapi.yaml` at the
//! repository root. Run after changing a route or a REST type:
//!
//!     cargo run -p sandbox-server --bin gen_openapi

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openapi/v1");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("openapi.json"), sandbox_server::openapi::json()?)?;
    std::fs::write(dir.join("openapi.yaml"), sandbox_server::openapi::yaml()?)?;
    println!("wrote {}", dir.display());
    Ok(())
}
