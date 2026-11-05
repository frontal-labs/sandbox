import json

import httpx
import pytest

from sandbox_sdk import errors
from sandbox_sdk.tier1 import Tier1Client


def client_with(handler) -> Tier1Client:
    return Tier1Client(
        "http://sandbox.test/",
        "key",
        transport=httpx.MockTransport(handler),
    )


@pytest.mark.asyncio
async def test_undo_posts_the_step_count_with_the_key():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["url"] = str(request.url)
        seen["auth"] = request.headers["authorization"]
        seen["body"] = json.loads(request.content)
        return httpx.Response(200, json={"root": "ab", "checkpoints": {"count": 2}})

    async with client_with(handler) as client:
        result = await client.undo("01SESSION", 2)

    assert result["checkpoints"]["count"] == 2
    assert seen["url"] == "http://sandbox.test/v1/sessions/01SESSION/undo"
    assert seen["auth"] == "Bearer key"
    assert seen["body"] == {"steps": 2}


@pytest.mark.asyncio
async def test_a_server_error_raises_the_matching_sandbox_error():
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            400, json={"code": "INVALID_ARGUMENT", "message": "only 0 checkpoint(s)"}
        )

    async with client_with(handler) as client:
        with pytest.raises(errors.SandboxError) as caught:
            await client.undo("01SESSION")

    assert caught.value.code == "INVALID_ARGUMENT"
    assert "only 0 checkpoint(s)" in str(caught.value)


@pytest.mark.asyncio
async def test_fork_command_returns_the_children():
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={"children": [{"session_id": "01CHILD", "exit_code": 0, "changed_files": 1}]},
        )

    async with client_with(handler) as client:
        children = await client.fork_command("01SESSION", 1, ["touch", "/r"])

    assert children[0]["session_id"] == "01CHILD"
