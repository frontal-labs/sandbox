import httpx
import pytest

from sandbox_sdk import errors
from sandbox_sdk.client import Client, ClientConfig, _encode_path


def test_dot_segments_are_refused():
  with pytest.raises(errors.SandboxError) as caught:
    _encode_path("a/../../snapshots")
  assert caught.value.code == "INVALID_ARGUMENT"
  with pytest.raises(errors.SandboxError):
    _encode_path("./x")


def test_segments_and_the_session_id_are_percent_encoded():
  assert _encode_path("dir name/file.txt") == "dir%20name/file.txt"
  assert _encode_path("a?b/c#d") == "a%3Fb/c%23d"


@pytest.mark.asyncio
async def test_a_refused_path_sends_no_request():
  seen = []

  def handler(request: httpx.Request) -> httpx.Response:
    seen.append(request)
    return httpx.Response(200, content=b"x")

  client = Client(ClientConfig(base_url="http://sandbox.test"))
  client._client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
  result = await client.read_file("01SESSION", "a/../../snapshots")

  assert isinstance(result, errors.SandboxError)
  assert result.code == "INVALID_ARGUMENT"
  assert seen == []
