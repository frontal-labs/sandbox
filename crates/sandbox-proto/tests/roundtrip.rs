//! Property and negative tests for the domain <-> proto conversions.

use proptest::prelude::*;
use prost_types::Timestamp;
use sandbox_proto::status::{from_status, to_status, ALL_ERROR_CODES};
use sandbox_proto::v1::types as pb;
use sandbox_types::event::{Event, FileOp};
use sandbox_types::hash::ContentHash;
use sandbox_types::policy::{NetworkPolicy, NetworkPolicyRule, PolicyAction, ResourceLimits};
use sandbox_types::session::{
    CheckpointPolicy, SessionConfig, SessionState, SessionStatus, Snapshot, Tier, TierPreference,
};
use sandbox_types::{
    ErrorCode, ExecRequest, ExecResult, SandboxError, SessionId, SnapshotId, TenantId,
};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ulid::Ulid;

fn arb_ulid() -> impl Strategy<Value = Ulid> {
    any::<u128>().prop_map(Ulid::from)
}

fn arb_hash() -> impl Strategy<Value = ContentHash> {
    any::<[u8; 32]>().prop_map(|bytes| ContentHash::from_hash(blake3::Hash::from_bytes(bytes)))
}

fn arb_time() -> impl Strategy<Value = SystemTime> {
    prop_oneof![
        (0u64..4_000_000_000, 0u32..1_000_000_000)
            .prop_map(|(s, n)| UNIX_EPOCH + Duration::new(s, n)),
        (1u64..1_000_000_000, 0u32..1_000_000_000)
            .prop_map(|(s, n)| UNIX_EPOCH - Duration::new(s, n)),
    ]
}

fn arb_map() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map("[a-zA-Z_][a-zA-Z0-9_]{0,8}", ".{0,12}", 0..4)
}

fn arb_tier() -> impl Strategy<Value = Tier> {
    prop_oneof![Just(Tier::Isolate), Just(Tier::Vm)]
}

fn arb_policy_rule() -> impl Strategy<Value = NetworkPolicyRule> {
    (
        prop_oneof![Just(PolicyAction::Allow), Just(PolicyAction::Deny)],
        "[a-z*.]{1,12}",
        // None (all ports), Some(empty) (no port), and Some(list) must all survive.
        prop::option::of(prop::collection::vec(any::<u16>(), 0..4)),
    )
        .prop_map(|(action, host_pattern, ports)| NetworkPolicyRule {
            action,
            host_pattern,
            ports,
        })
}

fn arb_limits() -> impl Strategy<Value = ResourceLimits> {
    (
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u64>()),
        prop::option::of(any::<u32>()),
        prop::option::of(any::<u32>()),
        prop::option::of(any::<u32>()),
    )
        .prop_map(|(a, b, c, d, e, f, g, h, i)| ResourceLimits {
            memory_bytes: a,
            cpu_ms_per_sec: b,
            wall_timeout_secs: c,
            max_file_size: d,
            max_total_size: e,
            max_inodes: f,
            max_path_depth: g,
            max_processes: h,
            max_open_files: i,
        })
}

fn arb_session_config() -> impl Strategy<Value = SessionConfig> {
    (
        arb_ulid(),
        prop_oneof![
            Just(TierPreference::Auto),
            Just(TierPreference::Isolate),
            Just(TierPreference::Vm)
        ],
        arb_limits(),
        any::<bool>(),
        prop::collection::vec(arb_policy_rule(), 0..3),
        arb_map(),
        "/[a-z/]{0,12}",
        prop::option::of(arb_ulid()),
        arb_map(),
        prop_oneof![
            Just(CheckpointPolicy::PerExec),
            Just(CheckpointPolicy::Manual)
        ],
        0u32..=200,
    )
        .prop_map(
            |(
                tenant,
                tier,
                limits,
                default_deny,
                rules,
                env,
                cwd,
                snap,
                labels,
                checkpoint,
                checkpoint_retain,
            )| SessionConfig {
                tenant_id: TenantId::from_ulid(tenant),
                tier_preference: tier,
                limits,
                network: NetworkPolicy {
                    default_deny,
                    rules,
                },
                env,
                cwd,
                initial_snapshot: snap.map(SnapshotId::from_ulid),
                secrets: None,
                labels,
                checkpoint,
                checkpoint_retain,
            },
        )
}

fn arb_exec_request() -> impl Strategy<Value = ExecRequest> {
    (
        prop::collection::vec(".{1,8}", 1..4),
        arb_map(),
        prop::option::of("/[a-z]{0,6}"),
        prop::option::of(prop::collection::vec(any::<u8>(), 0..16)),
        prop::option::of(any::<u64>()),
        any::<bool>(),
        prop::option::of(any::<u64>()),
    )
        .prop_map(
            |(argv, env, cwd, stdin, timeout_ms, stream, output_limit)| ExecRequest {
                argv,
                env,
                cwd,
                stdin,
                timeout_ms,
                stream,
                output_limit,
            },
        )
}

fn arb_exec_result() -> impl Strategy<Value = ExecResult> {
    (
        any::<i32>(),
        prop::collection::vec(any::<u8>(), 0..32),
        prop::collection::vec(any::<u8>(), 0..32),
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(|(exit_code, stdout, stderr, cpu_ms, wall_ms)| ExecResult {
            exit_code,
            stdout,
            stderr,
            cpu_ms,
            wall_ms,
        })
}

fn arb_sid() -> impl Strategy<Value = SessionId> {
    arb_ulid().prop_map(SessionId::from_ulid)
}

fn arb_event() -> impl Strategy<Value = Event> {
    prop_oneof![
        (arb_sid(), arb_ulid(), arb_tier(), arb_hash(), arb_time()).prop_map(
            |(s, t, tier, h, ts)| {
                Event::SessionCreated {
                    session_id: s,
                    tenant_id: TenantId::from_ulid(t),
                    tier,
                    config_hash: h,
                    timestamp: ts,
                }
            }
        ),
        (
            arb_sid(),
            any::<u64>(),
            prop::collection::vec(".{0,6}", 0..3),
            "/[a-z]{0,5}",
            arb_hash(),
            arb_time()
        )
            .prop_map(|(s, seq, argv, cwd, h, ts)| Event::ExecStarted {
                session_id: s,
                seq,
                argv,
                cwd,
                env_hash: h,
                timestamp: ts,
            }),
        (
            arb_sid(),
            any::<u64>(),
            any::<i32>(),
            any::<u64>(),
            any::<u64>(),
            arb_time()
        )
            .prop_map(
                |(s, seq, exit_code, cpu_ms, wall_ms, ts)| Event::ExecExited {
                    session_id: s,
                    seq,
                    exit_code,
                    cpu_ms,
                    wall_ms,
                    timestamp: ts,
                }
            ),
        (
            arb_sid(),
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..8),
            arb_time()
        )
            .prop_map(|(s, seq, data, ts)| Event::StdoutChunk {
                session_id: s,
                seq,
                data,
                timestamp: ts,
            }),
        (
            arb_sid(),
            any::<u64>(),
            "/[a-z]{0,5}",
            prop_oneof![
                Just(FileOp::Create),
                Just(FileOp::Write),
                Just(FileOp::Delete),
                Just(FileOp::Rename),
                Just(FileOp::Mkdir),
                Just(FileOp::Symlink)
            ],
            prop::option::of(arb_hash()),
            prop::option::of(arb_hash()),
            arb_time()
        )
            .prop_map(|(s, seq, path, op, before, after, ts)| Event::FileChanged {
                session_id: s,
                seq,
                path,
                op,
                before_hash: before,
                after_hash: after,
                timestamp: ts,
            }),
        (
            arb_sid(),
            any::<u64>(),
            "[a-z.]{0,8}",
            prop::option::of(0u16..=u16::MAX),
            any::<u64>(),
            arb_time()
        )
            .prop_map(|(s, seq, host, status, bytes, ts)| Event::NetRequest {
                session_id: s,
                seq,
                host,
                method: "GET".to_string(),
                status,
                request_bytes: bytes,
                response_bytes: bytes,
                timestamp: ts,
            }),
        (arb_sid(), arb_ulid(), arb_hash(), arb_time()).prop_map(|(s, snap, h, ts)| {
            Event::SnapshotTaken {
                session_id: s,
                snapshot_id: SnapshotId::from_ulid(snap),
                parent: None,
                vfs_root: h,
                timestamp: ts,
            }
        }),
        (arb_sid(), arb_tier(), arb_tier(), arb_time()).prop_map(|(s, from, to, ts)| {
            Event::Promoted {
                session_id: s,
                from_tier: from,
                to_tier: to,
                timestamp: ts,
            }
        }),
        (arb_sid(), arb_time()).prop_map(|(s, ts)| Event::SessionDestroyed {
            session_id: s,
            timestamp: ts,
        }),
    ]
}

fn assert_round_trip<D, P>(domain: D)
where
    D: Clone + PartialEq + std::fmt::Debug,
    P: From<D>,
    D: TryFrom<P, Error = SandboxError>,
{
    let wire: P = domain.clone().into();
    let back = D::try_from(wire).expect("valid wire value must convert back");
    assert_eq!(back, domain);
}

proptest! {
    #[test]
    fn exec_request_round_trips(req in arb_exec_request()) {
        assert_round_trip::<ExecRequest, pb::ExecRequest>(req);
    }

    #[test]
    fn exec_result_round_trips(res in arb_exec_result()) {
        assert_round_trip::<ExecResult, pb::ExecResult>(res);
    }

    #[test]
    fn session_config_round_trips(config in arb_session_config()) {
        assert_round_trip::<SessionConfig, pb::SessionConfig>(config);
    }

    #[test]
    fn session_state_round_trips(
        id in arb_ulid(), tenant in arb_ulid(), tier in arb_tier(), hash in arb_hash(),
        vfs in prop::option::of(arb_hash()), created in arb_time(), updated in arb_time(), labels in arb_map()
    ) {
        let state = SessionState {
            id: SessionId::from_ulid(id),
            tenant_id: TenantId::from_ulid(tenant),
            tier,
            status: SessionStatus::Running,
            config_hash: hash,
            vfs_root: vfs,
            created_at: created,
            updated_at: updated,
            labels,
        };
        assert_round_trip::<SessionState, pb::SessionState>(state);
    }

    #[test]
    fn snapshot_round_trips(
        id in arb_ulid(), parent in prop::option::of(arb_ulid()), vfs in arb_hash(), env in arb_map(),
        cwd in "/[a-z]{0,5}", tier in arb_tier(), blob in prop::option::of(arb_hash()), created in arb_time(), labels in arb_map()
    ) {
        let snapshot = Snapshot {
            id: SnapshotId::from_ulid(id),
            parent: parent.map(SnapshotId::from_ulid),
            vfs_root: vfs,
            env,
            cwd,
            tier,
            runtime_blob: blob,
            created_at: created,
            labels,
        };
        assert_round_trip::<Snapshot, pb::Snapshot>(snapshot);
    }

    #[test]
    fn event_round_trips(event in arb_event()) {
        assert_round_trip::<Event, pb::Event>(event);
    }

    #[test]
    fn content_hash_round_trips(hash in arb_hash()) {
        assert_round_trip::<ContentHash, pb::ContentHash>(hash);
    }

    #[test]
    fn ids_round_trip(raw in arb_ulid()) {
        assert_round_trip::<SessionId, pb::SessionId>(SessionId::from_ulid(raw));
        assert_round_trip::<SnapshotId, pb::SnapshotId>(SnapshotId::from_ulid(raw));
        assert_round_trip::<TenantId, pb::TenantId>(TenantId::from_ulid(raw));
    }

    #[test]
    fn error_codes_survive_a_status_round_trip(index in 0..ALL_ERROR_CODES.len(), message in ".{0,20}") {
        let code = ALL_ERROR_CODES[index];
        let error = SandboxError::new(code, message.clone());
        let back = from_status(&to_status(&error));
        prop_assert_eq!(back.code, code);
        prop_assert_eq!(back.message, message);
    }
}

#[test]
fn every_error_code_has_one_proto_value_and_back() {
    let mut seen = std::collections::HashSet::new();
    for code in ALL_ERROR_CODES {
        let wire: pb::ErrorCode = (*code).into();
        assert!(
            seen.insert(wire as i32),
            "duplicate proto value for {code:?}"
        );
        assert_eq!(ErrorCode::try_from(wire).unwrap(), *code);
    }
    // Keep ALL_ERROR_CODES in step with the domain enum: 21 variants today.
    assert_eq!(ALL_ERROR_CODES.len(), 21);
    assert_eq!(seen.len(), 21);
}

// --- Hostile and malformed wire input must be rejected, not defaulted -----

fn invalid_code(err: &SandboxError) -> bool {
    err.code == ErrorCode::InvalidArgument
}

#[test]
fn malformed_ids_are_rejected() {
    let bad = pb::SessionId {
        value: "not-a-ulid".to_string(),
    };
    assert!(invalid_code(&SessionId::try_from(bad).unwrap_err()));
}

#[test]
fn wrong_length_hash_is_rejected() {
    for len in [0usize, 31, 33] {
        let bad = pb::ContentHash {
            value: vec![0u8; len],
        };
        assert!(
            invalid_code(&ContentHash::try_from(bad).unwrap_err()),
            "len {len}"
        );
    }
}

#[test]
fn unspecified_enums_are_rejected() {
    assert!(invalid_code(
        &Tier::try_from(pb::Tier::Unspecified).unwrap_err()
    ));
    assert!(invalid_code(
        &TierPreference::try_from(pb::TierPreference::Unspecified).unwrap_err()
    ));
    assert!(invalid_code(
        &SessionStatus::try_from(pb::SessionStatus::Unspecified).unwrap_err()
    ));
    assert!(invalid_code(
        &PolicyAction::try_from(pb::PolicyAction::Unspecified).unwrap_err()
    ));
    assert!(invalid_code(
        &ErrorCode::try_from(pb::ErrorCode::Unspecified).unwrap_err()
    ));
}

#[test]
fn unknown_enum_numbers_are_rejected() {
    let bad = pb::SessionState {
        id: Some(pb::SessionId {
            value: Ulid::new().to_string(),
        }),
        tenant_id: Some(pb::TenantId {
            value: Ulid::new().to_string(),
        }),
        tier: 999,
        status: 2,
        config_hash: Some(ContentHash::new(b"x").into()),
        vfs_root: None,
        created_at: Some(Timestamp::default()),
        updated_at: Some(Timestamp::default()),
        labels: Default::default(),
    };
    assert!(invalid_code(&SessionState::try_from(bad).unwrap_err()));
}

#[test]
fn out_of_range_ports_are_rejected() {
    let bad = pb::NetworkPolicyRule {
        action: pb::PolicyAction::Allow as i32,
        host_pattern: "example.com".to_string(),
        ports: Some(pb::PortList {
            ports: vec![70_000],
        }),
    };
    assert!(invalid_code(&NetworkPolicyRule::try_from(bad).unwrap_err()));
}

#[test]
fn empty_port_list_stays_distinct_from_all_ports() {
    // Regression guard: an empty list must not turn into "every port".
    let none = pb::NetworkPolicyRule {
        action: pb::PolicyAction::Allow as i32,
        host_pattern: "h".to_string(),
        ports: Some(pb::PortList { ports: vec![] }),
    };
    let rule = NetworkPolicyRule::try_from(none).unwrap();
    assert_eq!(rule.ports, Some(vec![]));

    let all = pb::NetworkPolicyRule {
        action: pb::PolicyAction::Allow as i32,
        host_pattern: "h".to_string(),
        ports: None,
    };
    assert_eq!(NetworkPolicyRule::try_from(all).unwrap().ports, None);
}

#[test]
fn empty_argv_is_rejected() {
    let bad = pb::ExecRequest {
        argv: vec![],
        ..Default::default()
    };
    assert!(invalid_code(&ExecRequest::try_from(bad).unwrap_err()));
}

#[test]
fn missing_required_fields_are_rejected() {
    let event = pb::Event { kind: None };
    assert!(invalid_code(&Event::try_from(event).unwrap_err()));

    let config = pb::SessionConfig::default();
    assert!(invalid_code(&SessionConfig::try_from(config).unwrap_err()));
}

#[test]
fn timestamps_outside_the_nanosecond_range_are_rejected() {
    let bad = Timestamp {
        seconds: 0,
        nanos: 2_000_000_000,
    };
    assert!(invalid_code(
        &sandbox_proto::convert::time::from_timestamp(&bad).unwrap_err()
    ));
}

#[test]
fn unset_limits_stay_unset() {
    let limits = ResourceLimits {
        memory_bytes: None,
        cpu_ms_per_sec: None,
        wall_timeout_secs: None,
        max_file_size: None,
        max_total_size: None,
        max_inodes: None,
        max_path_depth: None,
        max_processes: None,
        max_open_files: None,
    };
    let back: ResourceLimits = pb::ResourceLimits::from(limits.clone()).into();
    assert_eq!(back, limits);
}

#[test]
fn default_config_round_trips_exactly() {
    let config = SessionConfig::new(TenantId::new());
    assert_round_trip::<SessionConfig, pb::SessionConfig>(config.clone());
    let _ = NetworkPolicy::default(); // default is deny-by-default: no rules
}
