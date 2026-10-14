//! Shared patterns for OpenAPI schemas. Compiled only with the `openapi` feature.

/// ULID text: 26 Crockford base32 characters.
pub const ULID_PATTERN: &str = "^[0-9A-HJKMNP-TV-Z]{26}$";
/// BLAKE3 hash as lowercase hex: 64 characters.
pub const HASH_PATTERN: &str = "^[0-9a-f]{64}$";
