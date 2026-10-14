//! Generated protobuf and gRPC bindings for the sandbox wire contracts.
//!
//! The `.proto` sources live in `proto/sandbox/v1/`. Modules here mirror the
//! proto packages: `sandbox.v1.types`, `sandbox.v1.api`, `sandbox.v1.worker`,
//! and `sandbox.v1.guest`. Conversions to and from `sandbox-types` live in
//! [`convert`]. Error mapping lives in [`status`]. Length-prefixed framing for
//! byte streams lives in [`framing`].

pub mod convert;
pub mod framing;
pub mod status;

/// The compiled file descriptor set, for schema checks.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.bin"));

pub mod v1 {
    pub mod types {
        tonic::include_proto!("sandbox.v1.types");
    }
    pub mod api {
        tonic::include_proto!("sandbox.v1.api");
    }
    pub mod worker {
        tonic::include_proto!("sandbox.v1.worker");
    }
    pub mod guest {
        tonic::include_proto!("sandbox.v1.guest");
    }
}
