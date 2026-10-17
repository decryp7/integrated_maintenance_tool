//! The gRPC transport against an in-process server on a private local socket: handshake, token, streaming,
//! cancellation, typed errors, panic containment, and what a broken peer looks like.
#![cfg(unix)]

use imt_ai::worker::endpoint::{new_token, PrivateEndpoint};
use imt_ai::worker::grpc_server::serve;
use imt_ai::worker::protocol::*;
use imt_ai::worker::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct Backend {
    loaded: bool,
}

impl WorkerBackend for Backend {
    fn load(&mut self, p: &LoadParams) -> Result<LoadResult, RpcError> {
        match p.model_path.as_str() {
            "bad" => Err(RpcError::new(code::LOAD_FAILED, "no such model — café")),
            "panic" => panic!("native code blew up"),
            _ => {
                self.loaded = true;
                Ok(LoadResult {
                    backend: "CPU".into(),
                    context: p.context,
                    name: "m".into(),
                    rss_bytes: 7,
                })
            }
        }
    }
    fn chat(&mut self, p: &ChatParams, delta: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> Result<ChatResult, RpcError> {
        match p.prompt.as_str() {
            "overflow" => return Err(context_overflow(9000, 8192)),
            "panic" => panic!("boom"),
            "schema" => {
                let s = p.json_schema.as_ref().map(|s| s.to_string()).unwrap_or_default();
                delta(&s);
                return Ok(ChatResult {
                    text: s,
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    finish: WireFinish::Stop,
                });
            }
            _ => {}
        }
        let mut text = String::new();
        for i in 0..40 {
            if cancelled() {
                return Ok(ChatResult {
                    text,
                    prompt_tokens: 3,
                    completion_tokens: i,
                    finish: WireFinish::Cancelled,
                });
            }
            std::thread::sleep(Duration::from_millis(if p.prompt == "slow" { 20 } else { 0 }));
            let piece = format!("w{i} ");
            delta(&piece);
            text.push_str(&piece);
        }
        Ok(ChatResult {
            text,
            prompt_tokens: 3,
            completion_tokens: 40,
            finish: WireFinish::Length,
        })
    }
    fn unload(&mut self) {
        self.loaded = false;
    }
}

struct Server {
    address: String,
    token: String,
    _endpoint: PrivateEndpoint,
    done: std::thread::JoinHandle<std::io::Result<()>>,
}

fn start() -> Server {
    let endpoint = PrivateEndpoint::create().unwrap();
    let token = new_token().unwrap();
    let (address, t) = (endpoint.address.clone(), token.clone());
    let done = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(serve(
            || Box::new(Backend { loaded: false }),
            &address,
            t,
            std::future::pending::<()>(),
        ))
    });
    let s = Server {
        address: endpoint.address.clone(),
        token,
        _endpoint: endpoint,
        done,
    };
    let end = Instant::now() + Duration::from_secs(5);
    while !std::path::Path::new(&s.address).exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    s
}

fn opts() -> CallOpts {
    CallOpts {
        timeout: Duration::from_secs(5),
        cancel_grace: Duration::from_secs(2),
    }
}

fn call(t: &mut GrpcTransport, m: &str, p: serde_json::Value) -> Result<serde_json::Value, TransportError> {
    t.call(m, p, opts(), None, &mut |_| {})
}

fn hello() -> serde_json::Value {
    json!(HelloParams {
        protocol_version: PROTOCOL_VERSION,
        client: "test".into()
    })
}

fn load(model: &str) -> serde_json::Value {
    json!(LoadParams {
        model_path: model.into(),
        gpu_layers: 0,
        context: 2048,
        threads: 0
    })
}

fn chat(prompt: &str) -> serde_json::Value {
    json!(ChatParams {
        prompt: prompt.into(),
        max_tokens: 100,
        temperature: 0.0,
        json_schema: None
    })
}

#[test]
fn a_full_conversation_streams_deltas_and_ends_with_a_result() {
    let s = start();
    let mut t = GrpcTransport::connect(&s.address, &s.token).unwrap();
    assert!(t.pid().is_none());
    let h: HelloResult = serde_json::from_value(call(&mut t, "hello", hello()).unwrap()).unwrap();
    assert_eq!((h.protocol_version, h.pid), (PROTOCOL_VERSION, std::process::id()));
    let st: StatusResult = serde_json::from_value(call(&mut t, "status", json!({})).unwrap()).unwrap();
    assert!(!st.loaded && st.rss_bytes > 0);
    let l: LoadResult = serde_json::from_value(call(&mut t, "load", load("ok")).unwrap()).unwrap();
    assert_eq!((l.backend.as_str(), l.context, l.name.as_str()), ("CPU", 2048, "m"));
    let mut deltas = Vec::new();
    let r = t
        .call("chat", chat("go"), opts(), None, &mut |d| {
            deltas.push(d["text"].as_str().unwrap().to_string())
        })
        .unwrap();
    let r: ChatResult = serde_json::from_value(r).unwrap();
    assert_eq!((r.completion_tokens, r.finish), (40, WireFinish::Length));
    assert_eq!(deltas.len(), 40);
    assert_eq!(deltas.concat(), r.text);
    // the JSON schema reaches the backend as the model sees it
    let schema = json!({"type": "object"});
    let r: ChatResult = serde_json::from_value(
        t.call(
            "chat",
            json!(ChatParams {
                prompt: "schema".into(),
                max_tokens: 5,
                temperature: 0.0,
                json_schema: Some(schema)
            }),
            opts(),
            None,
            &mut |_| {},
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(r.text, "{\"type\":\"object\"}");
    let st: StatusResult = serde_json::from_value(call(&mut t, "status", json!({})).unwrap()).unwrap();
    assert!(st.loaded && st.backend == "CPU");
    call(&mut t, "unload", json!({})).unwrap();
    assert!(matches!(call(&mut t, "chat", chat("go")), Err(TransportError::Remote(e)) if e.code == code::NOT_LOADED));
    assert!(matches!(call(&mut t, "nope", json!({})), Err(TransportError::Protocol(_))));
    assert!(matches!(
        call(&mut t, "load", json!("not params")),
        Err(TransportError::Protocol(_))
    ));
    call(&mut t, "shutdown", json!({})).unwrap();
    assert!(s.done.join().unwrap().is_ok(), "the server ends cleanly on Shutdown");
}

#[test]
fn every_call_needs_the_token_and_a_hello_first() {
    let s = start();
    let mut wrong = GrpcTransport::connect(&s.address, "not-the-token").unwrap();
    for m in ["hello", "status"] {
        let e = call(&mut wrong, m, if m == "hello" { hello() } else { json!({}) }).unwrap_err();
        assert!(
            matches!(&e, TransportError::Protocol(m) if m.contains("rejected the session token")),
            "{m}: {e:?}"
        );
    }
    let mut ok = GrpcTransport::connect(&s.address, &s.token).unwrap();
    for (m, p) in [
        ("status", json!({})),
        ("load", load("ok")),
        ("chat", chat("x")),
        ("unload", json!({})),
    ] {
        assert!(
            matches!(call(&mut ok, m, p), Err(TransportError::Remote(e)) if e.code == code::INVALID_REQUEST),
            "{m} before hello"
        );
    }
    let bad_version = call(
        &mut ok,
        "hello",
        json!(HelloParams {
            protocol_version: 99,
            client: String::new()
        }),
    )
    .unwrap_err();
    assert!(matches!(bad_version, TransportError::Remote(e) if e.code == code::INVALID_PARAMS && e.message.contains("99")));
    call(&mut ok, "hello", hello()).unwrap();
    // a token with characters that cannot travel in a header is refused before any call
    assert!(GrpcTransport::connect(&s.address, "line\nbreak").is_err());
    call(&mut ok, "shutdown", json!({})).unwrap();
    s.done.join().unwrap().unwrap();
}

#[test]
fn the_socket_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let s = start();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let sock = std::path::Path::new(&s.address);
    assert_eq!(mode(sock), 0o600);
    assert_eq!(mode(sock.parent().unwrap()), 0o700);
    let mut t = GrpcTransport::connect(&s.address, &s.token).unwrap();
    call(&mut t, "shutdown", json!({})).unwrap();
}

#[test]
fn typed_errors_and_panics_cross_the_wire() {
    let s = start();
    let mut t = GrpcTransport::connect(&s.address, &s.token).unwrap();
    call(&mut t, "hello", hello()).unwrap();
    match call(&mut t, "load", load("bad")).unwrap_err() {
        TransportError::Remote(e) => assert_eq!((e.code, e.message.as_str()), (code::LOAD_FAILED, "no such model — café")),
        e => panic!("{e:?}"),
    }
    assert!(matches!(call(&mut t, "load", load("panic")), Err(TransportError::Remote(e)) if e.code == code::ENGINE));
    call(&mut t, "load", load("ok")).unwrap();
    match call(&mut t, "chat", chat("overflow")).unwrap_err() {
        TransportError::Remote(e) => assert_eq!((e.code, e.data.unwrap()["limit"].clone()), (code::CONTEXT_OVERFLOW, json!(8192))),
        e => panic!("{e:?}"),
    }
    assert!(matches!(call(&mut t, "chat", chat("panic")), Err(TransportError::Remote(e)) if e.code == code::ENGINE));
    // the worker survived both panics
    assert!(call(&mut t, "chat", chat("go")).is_ok());
    call(&mut t, "shutdown", json!({})).unwrap();
}

#[test]
fn cancel_stops_a_running_chat_and_a_deaf_worker_times_out() {
    let s = start();
    let mut t = GrpcTransport::connect(&s.address, &s.token).unwrap();
    call(&mut t, "hello", hello()).unwrap();
    call(&mut t, "load", load("ok")).unwrap();
    let cancel = AtomicBool::new(false);
    let mut n = 0;
    let started = Instant::now();
    let r: ChatResult = serde_json::from_value(
        t.call("chat", chat("slow"), opts(), Some(&cancel), &mut |_| {
            n += 1;
            if n == 3 {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(r.finish, WireFinish::Cancelled);
    assert!(r.completion_tokens < 40 && started.elapsed() < Duration::from_millis(700));
    // silence longer than the allowed gap is a hang
    let e = t
        .call(
            "chat",
            chat("slow"),
            CallOpts {
                timeout: Duration::from_millis(5),
                cancel_grace: Duration::from_millis(50),
            },
            None,
            &mut |_| {},
        )
        .unwrap_err();
    assert!(matches!(e, TransportError::Timeout), "{e:?}");
    call(&mut t, "shutdown", json!({})).unwrap();
}

#[test]
fn a_peer_that_does_not_speak_grpc_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        use std::io::Write;
        for conn in listener.incoming().flatten() {
            let mut c = conn;
            let _ = c.write_all(b"this is not HTTP/2, just garbage\n\x00\xff\xfe");
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    let r = GrpcTransport::connect(path.to_str().unwrap(), "token").and_then(|mut t| call(&mut t, "hello", hello()));
    assert!(
        matches!(
            r,
            Err(TransportError::Died(_) | TransportError::Protocol(_) | TransportError::Timeout)
        ),
        "{r:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    // nothing is listening at all
    assert!(matches!(
        GrpcTransport::connect(dir.path().join("none.sock").to_str().unwrap(), "t"),
        Err(TransportError::Died(_))
    ));
}
