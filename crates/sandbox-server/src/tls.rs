//! TLS termination for the sandbox HTTP API (PLAN §4 / launch gap #5).
//!
//! When `[server.tls] enabled = true` the server terminates TLS itself with
//! rustls, instead of leaving that to a reverse proxy. Optional mTLS is
//! enabled with `ca` + `require_client_auth = true`, which makes clients
//! present a certificate signed by the given CA.

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

/// TLS settings for the HTTP listener, from `[server.tls]` in config.toml.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
    /// CA bundle used to verify client certificates (mTLS).
    pub ca: Option<PathBuf>,
    pub require_client_auth: bool,
}

impl TlsConfig {
    /// Build a rustls acceptor from these settings. Fails when a required
    /// file is missing or unparsable, so the server refuses to start with a
    /// broken TLS config instead of silently serving plaintext.
    pub fn acceptor(&self) -> Result<TlsAcceptor, TlsError> {
        if !self.enabled {
            return Err(TlsError::Disabled);
        }
        let cert_path = self.cert.as_deref().ok_or(TlsError::MissingCert)?;
        let key_path = self.key.as_deref().ok_or(TlsError::MissingKey)?;
        let certs = load_certs(cert_path)?;
        let key = load_private_key(key_path)?;

        let config = if self.require_client_auth {
            let ca_path = self.ca.as_deref().ok_or(TlsError::MissingCa)?;
            let ca = load_certs(ca_path)?;
            let mut roots = RootCertStore::empty();
            for cert in ca {
                roots
                    .add(cert)
                    .map_err(|e| TlsError::ClientVerifier(e.to_string()))?;
            }
            let client_auth = WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .map_err(|e| TlsError::ClientVerifier(e.to_string()))?;
            ServerConfig::builder()
                .with_client_cert_verifier(client_auth)
                .with_single_cert(certs, key)?
        } else {
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)?
        };
        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

/// Errors while setting up TLS.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("TLS is not enabled")]
    Disabled,
    #[error("cert is required when [server.tls] enabled = true")]
    MissingCert,
    #[error("key is required when [server.tls] enabled = true")]
    MissingKey,
    #[error("ca is required when require_client_auth = true")]
    MissingCa,
    #[error("reading certificate {0}: {1}")]
    ReadCert(PathBuf, String),
    #[error("reading private key {0}: {1}")]
    ReadKey(PathBuf, String),
    #[error("parsing private key: {0}")]
    ParseKey(String),
    #[error("parsing the client CA: {0}")]
    ClientVerifier(String),
}

/// A TCP connection, optionally wrapped in a completed TLS handshake.
///
/// `hyper` cannot handshake itself without the `server` feature, so the
/// handshake is done here and the resulting stream is handed to a plain
/// HTTP/1 connection per accepted socket.
pub enum Conn {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Conn {
    /// Handshake (TLS only) and adapt the stream for `TokioIo`.
    pub async fn established(
        stream: TcpStream,
        acceptor: Option<TlsAcceptor>,
    ) -> Result<TokioIo<Conn>, io::Error> {
        let conn = match acceptor {
            Some(acceptor) => Conn::Tls(Box::new(
                acceptor
                    .accept(stream)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?,
            )),
            None => Conn::Plain(stream),
        };
        Ok(TokioIo::new(conn))
    }
}

impl AsyncRead for Conn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        match self.as_mut().get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        match self.as_mut().get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        match self.as_mut().get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        match self.as_mut().get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Serve a router over TCP with optional TLS termination. One task per
/// connection: TLS handshake (if configured), then an HTTP/1 conversation.
///
/// Returns when the listener is dropped (the server is shutting down).
pub async fn serve(
    listener: TcpListener,
    acceptor: Option<TlsAcceptor>,
    app: axum::Router,
) -> io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let io = match Conn::established(stream, acceptor.clone()).await {
            Ok(io) => io,
            Err(e) => {
                tracing::warn!("handshake from {peer} failed: {e}");
                continue;
            }
        };
        // One HTTP/1 conversation per connection; the router is cloned per
        // connection and driven by `service_fn`.
        let service = app.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let mut router = service.clone();
                router.call(req)
            });
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await
            {
                tracing::debug!("connection from {peer} ended: {e}");
            }
        });
    }
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let data =
        std::fs::read(path).map_err(|e| TlsError::ReadCert(path.to_path_buf(), e.to_string()))?;
    let mut certs = Vec::new();
    for cert in rustls_pemfile::certs(&mut data.as_slice()) {
        certs.push(cert.map_err(|e| TlsError::ReadCert(path.to_path_buf(), e.to_string()))?);
    }
    if certs.is_empty() {
        return Err(TlsError::ReadCert(
            path.to_path_buf(),
            "no PEM certificates found".to_string(),
        ));
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    let data = std::fs::read(path).map_err(|e| TlsError::ReadKey(path.to_path_buf(), e.to_string()))?;
    let key = rustls_pemfile::private_key(&mut data.as_slice())
        .map_err(|e| TlsError::ReadKey(path.to_path_buf(), e.to_string()))?
        .ok_or_else(|| {
            TlsError::ParseKey(
                "no PEM private key found (expected PKCS#8, RSA or EC)".to_string(),
            )
        })?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_rejects_acceptor() {
        let config = TlsConfig::default();
        assert!(matches!(config.acceptor(), Err(TlsError::Disabled)));
    }

    #[test]
    fn enabled_but_missing_cert_rejects() {
        let config = TlsConfig {
            enabled: true,
            cert: None,
            key: Some(PathBuf::from("key.pem")),
            ca: None,
            require_client_auth: false,
        };
        assert!(matches!(config.acceptor(), Err(TlsError::MissingCert)));
    }

    #[test]
    fn enabled_but_missing_key_rejects() {
        let config = TlsConfig {
            enabled: true,
            cert: Some(PathBuf::from("cert.pem")),
            key: None,
            ca: None,
            require_client_auth: false,
        };
        assert!(matches!(config.acceptor(), Err(TlsError::MissingKey)));
    }

    fn ensure_crypto_provider() {
        // Install the ring provider if not already set (no-op if already installed).
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    /// With a self-signed cert generated by openssl, the acceptor must build.
    #[test]
    fn self_signed_cert_builds_acceptor() {
        ensure_crypto_provider();
        let dir = std::env::temp_dir().join(format!("sandbox-tls-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        let status = std::process::Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-keyout",
                key.to_str().unwrap(),
                "-out",
                cert.to_str().unwrap(),
                "-days",
                "1",
                "-nodes",
                "-subj",
                "/CN=localhost",
            ])
            .status()
            .expect("openssl is available for tests");
        assert!(status.success(), "openssl failed: {status}");

        let config = TlsConfig {
            enabled: true,
            cert: Some(cert.clone()),
            key: Some(key.clone()),
            ca: None,
            require_client_auth: false,
        };
        let result = config.acceptor();
        assert!(result.is_ok(), "acceptor failed: {:?}", result.err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
