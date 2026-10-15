"""Client tests. The wire conversion is checked offline. The lifecycle tests run
against a live server when SANDBOX_BASE_URL and SANDBOX_API_KEY are set, as CI
does after starting `sandbox serve`."""

from __future__ import annotations

import asyncio
import os

import pytest

from sandbox_sdk.client import Client, ClientConfig, _exec_result, _wire
from sandbox_sdk.errors import is_sandbox_error
from sandbox_sdk.types import ExecRequest, NetworkPolicy, ResourceLimits, SessionConfig

TENANT = "00000000000000000000000000"


def test_wire_converts_nested_dataclasses_and_bytes():
    config = SessionConfig(
        tenant_id=TENANT,
        limits=ResourceLimits(memory_bytes=1024),
        network=NetworkPolicy.allow_all(),
    )
    wire = _wire(config)
    assert wire["limits"]["memory_bytes"] == 1024
    assert wire["network"] == {"default_deny": False, "rules": []}
    assert _wire(ExecRequest(argv=["cat"], stdin=b"\x01\x02")) == {
        "argv": ["cat"],
        "env": {},
        "cwd": None,
        "stdin": [1, 2],
        "timeout_ms": None,
        "stream": False,
    }


def test_exec_result_decodes_byte_arrays():
    result = _exec_result(
        {"exit_code": 0, "stdout": [104, 105, 10], "stderr": [], "cpu_ms": 0, "wall_ms": 3}
    )
    assert result.stdout == b"hi\n"
    assert result.stderr == b""


live = pytest.mark.skipif(
    not (os.environ.get("SANDBOX_BASE_URL") and os.environ.get("SANDBOX_API_KEY")),
    reason="needs SANDBOX_BASE_URL and SANDBOX_API_KEY",
)


def _client() -> Client:
    return Client(
        ClientConfig(
            base_url=os.environ["SANDBOX_BASE_URL"],
            api_key=os.environ["SANDBOX_API_KEY"],
        )
    )


@live
def test_session_exec_snapshot_sleep_and_wake_against_the_server():
    async def run() -> None:
        client = _client()
        try:
            created = await client.create_session(SessionConfig(tenant_id=TENANT))
            assert not is_sandbox_error(created), created
            session_id = created

            state = await client.get_session(session_id)
            assert state.status == "running"

            result = await client.exec(session_id, ExecRequest(argv=["echo", "hi"]))
            assert not is_sandbox_error(result), result
            assert result.exit_code == 0
            assert result.stdout == b"hi\n"

            snapshot = await client.snapshot(session_id)
            assert not is_sandbox_error(snapshot), snapshot
            assert snapshot.id

            sleep_id = await client.sleep(session_id)
            assert not is_sandbox_error(sleep_id), sleep_id
            assert (await client.get_session(session_id)).status == "sleeping"

            asleep = await client.exec(session_id, ExecRequest(argv=["true"]))
            assert is_sandbox_error(asleep)
            assert asleep.code == "INVALID_STATE"

            woken = await client.wake(session_id)
            assert not is_sandbox_error(woken), woken
            assert woken.status == "running"

            again = await client.exec(session_id, ExecRequest(argv=["echo", "back"]))
            assert again.stdout == b"back\n"

            assert await client.destroy_session(session_id) is None
        finally:
            await client.close()

    asyncio.run(run())
