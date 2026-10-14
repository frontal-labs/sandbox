//! Schema parity: every serde field of a domain type appears in its proto
//! message with the same name, and the reverse. The descriptor set is the one
//! generated from `proto/` at build time, so this compares the real schema.

use prost::Message;
use prost_types::{DescriptorProto, FileDescriptorSet};
use sandbox_types::policy::{NetworkPolicy, NetworkPolicyRule, ResourceLimits};
use sandbox_types::session::SessionConfig;
use sandbox_types::{ExecRequest, ExecResult, TenantId};
use std::collections::BTreeSet;

fn descriptor_set() -> FileDescriptorSet {
    FileDescriptorSet::decode(sandbox_proto::FILE_DESCRIPTOR_SET).expect("descriptor set decodes")
}

/// Field names of `message` in `package`, read from the compiled schema.
fn proto_fields(set: &FileDescriptorSet, package: &str, message: &str) -> BTreeSet<String> {
    fn find<'a>(msgs: &'a [DescriptorProto], name: &str) -> Option<&'a DescriptorProto> {
        msgs.iter().find(|m| m.name() == name)
    }
    for file in &set.file {
        if file.package() != package {
            continue;
        }
        if let Some(msg) = find(&file.message_type, message) {
            return msg.field.iter().map(|f| f.name().to_string()).collect();
        }
    }
    panic!("message {package}.{message} not found in the descriptor set");
}

/// Top-level keys that serde emits for a domain value (None fields included).
fn serde_keys<T: serde::Serialize>(value: &T) -> BTreeSet<String> {
    serde_json::to_value(value)
        .expect("domain value serializes")
        .as_object()
        .expect("domain struct serializes to an object")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn session_config_fields_match_the_proto() {
    let set = descriptor_set();
    let domain = serde_keys(&SessionConfig::new(TenantId::new()));
    assert_eq!(
        domain,
        proto_fields(&set, "sandbox.v1.types", "SessionConfig")
    );
}

#[test]
fn exec_request_fields_match_the_proto() {
    let set = descriptor_set();
    let domain = serde_keys(&ExecRequest::default());
    assert_eq!(
        domain,
        proto_fields(&set, "sandbox.v1.types", "ExecRequest")
    );
}

#[test]
fn exec_result_fields_match_the_proto() {
    let set = descriptor_set();
    let domain = serde_keys(&ExecResult {
        exit_code: 0,
        stdout: vec![],
        stderr: vec![],
        cpu_ms: 0,
        wall_ms: 0,
    });
    assert_eq!(domain, proto_fields(&set, "sandbox.v1.types", "ExecResult"));
}

#[test]
fn resource_limits_fields_match_the_proto() {
    let set = descriptor_set();
    let domain = serde_keys(&ResourceLimits::default());
    assert_eq!(
        domain,
        proto_fields(&set, "sandbox.v1.types", "ResourceLimits")
    );
}

#[test]
fn network_policy_fields_match_the_proto() {
    let set = descriptor_set();
    assert_eq!(
        serde_keys(&NetworkPolicy::default()),
        proto_fields(&set, "sandbox.v1.types", "NetworkPolicy")
    );
    let rule = NetworkPolicyRule::allow_http("example.com");
    assert_eq!(
        serde_keys(&rule),
        proto_fields(&set, "sandbox.v1.types", "NetworkPolicyRule")
    );
}

#[test]
fn every_service_is_in_the_descriptor_set() {
    let set = descriptor_set();
    let services: BTreeSet<String> = set
        .file
        .iter()
        .flat_map(|f| {
            f.service
                .iter()
                .map(move |s| format!("{}.{}", f.package(), s.name()))
        })
        .collect();
    for expected in [
        "sandbox.v1.api.SessionService",
        "sandbox.v1.api.ExecService",
        "sandbox.v1.api.FsService",
        "sandbox.v1.api.SnapshotService",
        "sandbox.v1.api.UsageService",
        "sandbox.v1.worker.WorkerService",
        "sandbox.v1.guest.GuestService",
    ] {
        assert!(services.contains(expected), "missing service {expected}");
    }
}
