//! HTTP client for `sandbox-server`. Every method is one request to the `/v1`
//! API, and every error the server returns maps back to a `SandboxError` with
//! the same code.

use reqwest::{Method, StatusCode};
use sandbox_types::{
    ErrorCode, ExecRequest, SandboxError, SessionConfig, SessionId, SessionState, Snapshot,
    SnapshotId,
};
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "http://localhost:8080";

/// Client for one server. Cheap to clone; clones share the connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

/// Output of a command, with the streams as text.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Deserialize)]
struct SessionIdBody {
    session_id: SessionId,
}

#[derive(Deserialize)]
struct SnapshotIdBody {
    snapshot_id: SnapshotId,
}

#[derive(Deserialize)]
struct SessionIdsBody {
    session_ids: Vec<SessionId>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

#[derive(Serialize)]
struct ForkBody {
    count: u32,
}

impl Client {
    /// A client for the server at `base_url`. `api_key` is sent as a bearer token.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// A client configured from `SANDBOX_BASE_URL` (default `http://localhost:8080`)
    /// and `SANDBOX_API_KEY`.
    pub fn from_env() -> Result<Self, SandboxError> {
        let base_url =
            std::env::var("SANDBOX_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let api_key = std::env::var("SANDBOX_API_KEY").ok();
        Ok(Self::new(base_url, api_key))
    }

    /// Creates a session owned by this client's key.
    pub async fn create_session(&self, config: SessionConfig) -> Result<SessionId, SandboxError> {
        let body: SessionIdBody = self
            .send_json(Method::POST, "/v1/sessions", Some(&config))
            .await?;
        Ok(body.session_id)
    }

    /// Runs `command`, split on whitespace, in the session.
    pub async fn exec(
        &self,
        session: SessionId,
        command: &str,
    ) -> Result<ExecResult, SandboxError> {
        let request = ExecRequest {
            argv: command.split_whitespace().map(str::to_string).collect(),
            ..ExecRequest::default()
        };
        let out = self.exec_request(session, &request).await?;
        Ok(ExecResult {
            exit_code: out.exit_code,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Runs a fully specified request in the session.
    pub async fn exec_request(
        &self,
        session: SessionId,
        request: &ExecRequest,
    ) -> Result<sandbox_types::ExecResult, SandboxError> {
        self.send_json(
            Method::POST,
            &format!("/v1/sessions/{session}/exec"),
            Some(request),
        )
        .await
    }

    /// Reads a file from the session's filesystem.
    pub async fn read_file(&self, session: SessionId, path: &str) -> Result<Vec<u8>, SandboxError> {
        let response = self
            .send(Method::GET, &file_url(session, path), None, None)
            .await?;
        response_bytes(response).await
    }

    /// Writes a file, creating it or replacing it.
    pub async fn write_file(
        &self,
        session: SessionId,
        path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), SandboxError> {
        self.send(
            Method::PUT,
            &file_url(session, path),
            Some(bytes),
            Some("application/octet-stream"),
        )
        .await
        .map(drop)
    }

    /// Deletes a file or an empty directory.
    pub async fn delete_file(&self, session: SessionId, path: &str) -> Result<(), SandboxError> {
        self.send(Method::DELETE, &file_url(session, path), None, None)
            .await
            .map(drop)
    }

    /// Snapshots the session's filesystem.
    pub async fn snapshot(&self, session: SessionId) -> Result<SnapshotId, SandboxError> {
        let body: SnapshotIdBody = self
            .send_json::<(), _>(
                Method::POST,
                &format!("/v1/sessions/{session}/snapshot"),
                None,
            )
            .await?;
        Ok(body.snapshot_id)
    }

    /// Puts the session to sleep and returns the snapshot it wakes from. A
    /// sleeping session holds no filesystem in memory until `wake` is called.
    pub async fn sleep(&self, session: SessionId) -> Result<SnapshotId, SandboxError> {
        let body: SnapshotIdBody = self
            .send_json::<(), _>(Method::POST, &format!("/v1/sessions/{session}/sleep"), None)
            .await?;
        Ok(body.snapshot_id)
    }

    /// Wakes a sleeping session. Its files come back from the sleep snapshot.
    /// Shell state starts fresh.
    pub async fn wake(&self, session: SessionId) -> Result<SessionState, SandboxError> {
        self.send_json::<(), _>(Method::POST, &format!("/v1/sessions/{session}/wake"), None)
            .await
    }

    /// Snapshots taken from the session.
    pub async fn list_snapshots(&self, session: SessionId) -> Result<Vec<Snapshot>, SandboxError> {
        self.send_json::<(), _>(
            Method::GET,
            &format!("/v1/sessions/{session}/snapshots"),
            None,
        )
        .await
    }

    /// One snapshot taken from the session.
    pub async fn get_snapshot(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> Result<Snapshot, SandboxError> {
        self.send_json::<(), _>(
            Method::GET,
            &format!("/v1/sessions/{session}/snapshots/{snapshot}"),
            None,
        )
        .await
    }

    /// Starts `count` sessions from a snapshot.
    pub async fn fork(
        &self,
        snapshot: SnapshotId,
        count: u32,
    ) -> Result<Vec<SessionId>, SandboxError> {
        let body: SessionIdsBody = self
            .send_json(
                Method::POST,
                &format!("/v1/snapshots/{snapshot}/fork"),
                Some(&ForkBody { count }),
            )
            .await?;
        Ok(body.session_ids)
    }

    /// Destroys a session.
    /// The current state of a session, sleeping or not.
    pub async fn get_session(&self, session: SessionId) -> Result<SessionState, SandboxError> {
        self.send_json::<(), _>(Method::GET, &format!("/v1/sessions/{session}"), None)
            .await
    }

    pub async fn destroy_session(&self, session: SessionId) -> Result<(), SandboxError> {
        self.send(
            Method::DELETE,
            &format!("/v1/sessions/{session}"),
            None,
            None,
        )
        .await
        .map(drop)
    }

    async fn send_json<B, T>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, SandboxError>
    where
        B: Serialize,
        T: serde::de::DeserializeOwned,
    {
        let bytes = match body {
            Some(body) => Some(serde_json::to_vec(body).map_err(|e| {
                SandboxError::new(ErrorCode::InvalidArgument, format!("encoding request: {e}"))
            })?),
            None => None,
        };
        let content_type = bytes.as_ref().map(|_| "application/json");
        let response = self.send(method, path, bytes, content_type).await?;
        let bytes = response_bytes(response).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| SandboxError::new(ErrorCode::Internal, format!("decoding response: {e}")))
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<reqwest::Response, SandboxError> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        if let Some(content_type) = content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(|e| {
            SandboxError::new(
                ErrorCode::Unavailable,
                format!("request to {path} failed: {e}"),
            )
        })?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(error_from_response(response).await)
    }
}

/// Turns a non-2xx response into the `SandboxError` the server meant. A body
/// without the error shape is mapped from the HTTP status.
async fn error_from_response(response: reqwest::Response) -> SandboxError {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if let Ok(body) = serde_json::from_slice::<ErrorBody>(&bytes) {
        return SandboxError::new(body.code, body.message);
    }
    let code = match status {
        StatusCode::UNAUTHORIZED => ErrorCode::PermissionDenied,
        StatusCode::NOT_FOUND => ErrorCode::NotFound,
        StatusCode::BAD_REQUEST => ErrorCode::InvalidArgument,
        StatusCode::NOT_IMPLEMENTED => ErrorCode::Unsupported,
        _ => ErrorCode::Internal,
    };
    SandboxError::new(code, format!("server answered {status}"))
}

async fn response_bytes(response: reqwest::Response) -> Result<Vec<u8>, SandboxError> {
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| SandboxError::new(ErrorCode::Unavailable, format!("reading response: {e}")))
}

fn file_url(session: SessionId, path: &str) -> String {
    let encoded: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(percent_encode)
        .collect();
    format!("/v1/sessions/{session}/fs/{}", encoded.join("/"))
}

/// Percent-encodes one path segment. Unreserved characters pass through.
fn percent_encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(percent_encode("a-b_c.txt"), "a-b_c.txt");
        assert_eq!(percent_encode("sp ace?#"), "sp%20ace%3F%23");
    }

    #[test]
    fn file_urls_encode_each_segment_and_keep_slashes() {
        let session = SessionId::new();
        let url = file_url(session, "/dir/my file.txt");
        assert_eq!(url, format!("/v1/sessions/{session}/fs/dir/my%20file.txt"));
    }

    #[test]
    fn base_url_loses_trailing_slashes() {
        let client = Client::new("http://example.test:8080/", None);
        assert_eq!(client.base_url, "http://example.test:8080");
    }
}
