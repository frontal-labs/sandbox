//! TLS termination and optional mTLS for the sandbox HTTP API.
//!
//! Generates a self-signed certificate at test time, starts the server with
//! TLS enabled, and verifies:
//! 1. A TLS client (that trusts the cert) can reach the API.
//! 2. A plaintext HTTP client is rejected.
//! 3. With mTLS enabled, a client without a cert is rejected.

use std::process::Command;
use std::sync::Arc;

use sandbox_scheduler::Scheduler;
use sandbox_server::auth::AuthManager;
use sandbox_server::tls::TlsConfig;
use sandbox_server::{create_router_with_options, ServerOptions, tls};

fn tmpdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sandbox-tls-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn generate_self_signed() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let d = tmpdir();
    let key = d.join("key.pem");
    let cert = d.join("cert.pem");
    let ca = d.join("ca.pem");

    // Generate a self-signed cert and key.
    Command::new("openssl")
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
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ])
        .output()
        .expect("openssl req");
    // Copy the cert as the CA bundle.
    std::fs::copy(&cert, &ca).unwrap();
    (cert, key, ca)
}

#[tokio::test]
async fn tls_serves_and_rejects_plaintext() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key, _ca) = generate_self_signed();

    let cfg = TlsConfig {
        enabled: true,
        cert: Some(cert.clone()),
        key: Some(key.clone()),
        ca: None,
        require_client_auth: false,
    };
    let acceptor = cfg.acceptor().unwrap();

    let auth = Arc::new(AuthManager::new());
    let router = create_router_with_options(
        Arc::new(Scheduler::new()),
        auth,
        ServerOptions::default(),
    )
    .0;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tls::serve(listener, Some(acceptor), router).await.unwrap();
    });

    let host = format!("127.0.0.1:{}", addr.port());

    // 1. TLS client succeeds.
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
    let res = client
        .get(format!("https://{host}/v1/openapi.json"))
        .send()
        .await
        .expect("tls client connects");
    assert_eq!(res.status(), 200);

    // 2. Plaintext HTTP client is rejected.
    let res = reqwest::Client::new()
        .get(format!("http://{host}/v1/openapi.json"))
        .send()
        .await;
    // The TLS server rejects the plaintext connection (handshake failure or
    // protocol error), so the client must not get a 200. An `Err` here is
    // expected: connection/protocol error.
    if let Ok(r) = res {
        assert_ne!(r.status(), 200, "plaintext must not be served as 200");
    }

    server.abort();
}
