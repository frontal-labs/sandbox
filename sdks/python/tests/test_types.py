"""
Tests for the sandbox SDK.
"""

from sandbox_sdk import (
  ExecRequest,
  ExecResult,
  NetworkPolicy,
  ResourceLimits,
  SessionConfig,
  Snapshot,
  generate_id,
  internal,
  invalid_state,
  is_sandbox_error,
  nil_id,
)


def test_session_config_builder() -> None:
  config = SessionConfig.new("tenant-123")
  assert config.tenant_id == "tenant-123"
  assert config.tier_preference == "auto"
  assert config.cwd == "/work"

  config2 = config.with_tier("isolate").with_cwd("/home").with_label("key", "value")
  assert config2.tier_preference == "isolate"
  assert config2.cwd == "/home"
  assert config2.labels == {"key": "value"}


def test_resource_limits() -> None:
  limits = ResourceLimits(memory_bytes=1024 * 1024 * 1024, cpu_ms_per_sec=500)
  assert limits.memory_bytes == 1024 * 1024 * 1024
  assert limits.cpu_ms_per_sec == 500


def test_network_policy() -> None:
  policy = NetworkPolicy.allow_all()
  assert policy.default_deny is False

  policy2 = policy.add_rule({"action": "allow", "host_pattern": "example.com", "ports": [80, 443]})
  assert len(policy2.rules) == 1
  assert policy2.rules[0]["host_pattern"] == "example.com"


def test_generate_id() -> None:
  id1 = generate_id()
  id2 = generate_id()
  assert len(id1) == 26
  assert id1 != id2


def test_nil_id() -> None:
  nil = nil_id()
  assert nil == "00000000000000000000000000"


def test_errors() -> None:
  err = internal("test error")
  assert err.code == "INTERNAL"
  assert str(err) == "[INTERNAL] test error"

  err2 = invalid_state("no session")
  assert err2.code == "INVALID_STATE"
  assert is_sandbox_error(err2)
  assert not is_sandbox_error(ValueError("test"))


def test_exec_request() -> None:
  req = ExecRequest(argv=["echo", "hello"], env={"FOO": "bar"}, stream=True)
  assert req.argv == ["echo", "hello"]
  assert req.env == {"FOO": "bar"}
  assert req.stream is True


def test_exec_result() -> None:
  result = ExecResult(exit_code=0, stdout=b"hello", stderr=b"", cpu_ms=10, wall_ms=20)
  assert result.exit_code == 0
  assert result.stdout == b"hello"
  assert result.cpu_ms == 10


def test_snapshot() -> None:
  snap = Snapshot(
    id="01ARZ3NDEKTSV4RRFFQ69G5FAV",
    parent=None,
    vfs_root="hash123",
    env={},
    cwd="/work",
    tier="isolate",
    runtime_blob=None,
    created_at="2024-01-01T00:00:00Z",
    labels={},
  )
  assert snap.id == "01ARZ3NDEKTSV4RRFFQ69G5FAV"
  assert snap.tier == "isolate"
