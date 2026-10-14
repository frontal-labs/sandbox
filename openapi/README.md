# openapi

The REST API contract, generated from the Rust code. The document is built by `crates/sandbox-server/src/openapi.rs`, from the `#[utoipa::path]` annotations in `api.rs` and the `ToSchema` derives in `sandbox-types` (feature `openapi`).

- `v1/openapi.json` and `v1/openapi.yaml`: generated and committed.
- Served live at `GET /v1/openapi.json`, without auth. The spec holds no secrets.

## Regenerate

After changing a route or a REST-facing type:

```sh
cargo run -p sandbox-server --bin gen_openapi
```

## Checks

- `cargo test -p sandbox-server --test openapi` fails when the committed files differ from the code, when a `$ref` dangles, or when a documented route is not routed (or a routed path is not documented).
- Validate with a standard validator, for example `openapi-spec-validator` (both files pass).

## Known gaps

- Routes that return 501 (snapshot and fork) are not implemented yet. Their responses say so.
- The Python and TypeScript SDKs also call `GET /v1/sessions/{id}/snapshots` and `GET /v1/sessions/{id}/snapshots/{snapshot_id}`. The server does not route them, so those calls return 404. They are not in this spec.
- Byte fields (`stdout`, `stderr`, `stdin`) are integer arrays, which is serde's default for `Vec<u8>`. The SDKs model them as bytes but do not decode them. Choosing a JSON encoding (for example base64) is a wire-format decision that is still open.
