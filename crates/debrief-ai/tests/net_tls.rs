//! A TLS-intercepting proxy, simulated: a local HTTPS server whose certificate no trust store knows (like a
//! Zscaler root that IT has not installed). Debrief's clients must refuse it, never fall back to an unverified
//! connection, and say in plain words what is wrong.
//!
//! `fixtures/tls/cert.der` is a self-signed certificate for `localhost` / 127.0.0.1 (valid until 2126) and
//! `key.der` its PKCS#8 P-256 key; test-only, generated with
//! `openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 36500 -subj /CN=localhost ...`.

use debrief_ai::models::{HfConfig, HuggingFace, ModelSource};
use debrief_ai::net::{self, HttpConfig, NetKind};
use debrief_ai::AiError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CERT: &[u8] = include_bytes!("fixtures/tls/cert.der");
const KEY: &[u8] = include_bytes!("fixtures/tls/key.der");

/// Serves HTTPS on a loopback port until the test ends; counts requests that got past the handshake.
fn https_server() -> (u16, Arc<AtomicUsize>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(CERT.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
        )
        .unwrap();
    let cfg = Arc::new(cfg);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicUsize::new(0));
    let count = served.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (cfg, count) = (cfg.clone(), count.clone());
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let Ok(conn) = rustls::ServerConnection::new(cfg) else { return };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut buf = [0u8; 4096];
                if matches!(tls.read(&mut buf), Ok(n) if n > 0) {
                    count.fetch_add(1, Ordering::SeqCst);
                    let _ = tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
                }
            });
        }
    });
    (port, served)
}

fn client() -> ureq::Agent {
    net::agent(&HttpConfig {
        proxy: None,
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(10),
    })
    .unwrap()
}

#[test]
fn an_unknown_root_certificate_is_refused_and_explained() {
    let (port, served) = https_server();
    let host = "localhost";
    let err = client()
        .get(&format!("https://{host}:{port}/model.gguf?sig=secret"))
        .call()
        .unwrap_err();
    let e = net::classify(err, host, false);
    match &e {
        AiError::Connection(n) => assert_eq!(n.kind, NetKind::Tls, "{e}"),
        other => panic!("not a certificate problem: {other:?}"),
    }
    let msg = e.to_string();
    assert!(
        msg.contains("Zscaler") && msg.contains("root certificate") && msg.contains("manually"),
        "{msg}"
    );
    assert!(!msg.contains("secret"));
    assert!(!e.is_transient(), "a certificate problem is not retried");
    assert_eq!(
        served.load(Ordering::SeqCst),
        0,
        "no request may be sent over an unverified connection"
    );
}

#[test]
fn the_model_download_reports_the_certificate_problem() {
    let (port, served) = https_server();
    // An https mirror endpoint on the interception server: the adapter's own request path.
    let mut cfg = HfConfig::default();
    cfg.endpoint = format!("https://localhost:{port}");
    cfg.connect_timeout = Duration::from_secs(5);
    cfg.api_timeout = Duration::from_secs(10);
    let hf = HuggingFace::new(cfg).unwrap();
    let e = hf.describe("org/repo", "m.gguf").unwrap_err();
    assert!(
        matches!(&e, AiError::Connection(n) if n.kind == NetKind::Tls && n.host == "localhost"),
        "{e:?}"
    );
    assert_eq!(served.load(Ordering::SeqCst), 0);
}

#[test]
fn nothing_listening_is_offline_and_an_unreachable_proxy_is_the_proxy() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let err = client().get(&format!("https://127.0.0.1:{port}/")).call().unwrap_err();
    let e = net::classify(err, "127.0.0.1", false);
    assert!(matches!(&e, AiError::Connection(n) if n.kind == NetKind::Offline), "{e:?}");
    assert!(e.is_transient());
    let via = net::agent(&HttpConfig {
        proxy: Some(format!("http://127.0.0.1:{port}")),
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(10),
    })
    .unwrap();
    let err = via.get("https://huggingface.co/").call().unwrap_err();
    let e = net::classify(err, "huggingface.co", true);
    assert!(matches!(&e, AiError::Connection(n) if n.kind == NetKind::Proxy), "{e:?}");
    assert!(e.to_string().contains("Download proxy"));
}

#[test]
fn a_proxy_that_wants_a_sign_in_is_reported_as_such() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"corp\"\r\ncontent-length: 0\r\n\r\n",
            );
            // Keep the connection until the client hangs up (closing first would reset it on Windows).
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            while matches!(s.read(&mut buf), Ok(n) if n > 0) {}
        }
    });
    let via = net::agent(&HttpConfig {
        proxy: Some(format!("http://127.0.0.1:{port}")),
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(10),
    })
    .unwrap();
    let err = via.get("https://github.com/").call().unwrap_err();
    let e = net::classify(err, "github.com", true);
    assert!(matches!(&e, AiError::Connection(n) if n.kind == NetKind::ProxyAuth), "{e:?}");
    assert!(!e.is_transient());
}
