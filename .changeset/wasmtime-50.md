---
"frontal-sandbox": patch
---

Bump `wasmtime` from 36.0.20 to 50.0.1 and remove the unused `wasmtime-wasi` dependency (confirmed no references in source or other `Cargo.toml` files). Wasmtime 50 raises the MSRV to rustc 1.97.0, so the Rust toolchain is bumped from 1.95 to 1.97 (`rust-toolchain.toml`, `Cargo.toml` `rust-version`, CI workflow comment). Update `plans/PLAN.md` dependency list to reflect the removal of `wasmtime-wasi`.
