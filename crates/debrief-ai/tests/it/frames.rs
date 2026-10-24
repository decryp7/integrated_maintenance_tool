//! The worker protocol over frames against an in-process worker: hello first, streaming, cancellation, typed
//! errors and panic containment. (Tokens, private endpoints, garbage and oversized frames are tested in debrief-ipc.)

use debrief_ai::worker::protocol::*;
use debrief_ai::worker::*;
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
                    vision: p.mmproj_path.is_some(),
                    server: None,
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

fn start() -> FramesTransport {
    FramesTransport::in_process(|| Box::new(Backend { loaded: false })).unwrap()
}

fn opts() -> CallOpts {
    CallOpts {
        timeout: Duration::from_secs(5),
        cancel_grace: Duration::from_secs(2),
    }
}

fn call(t: &mut FramesTransport, m: &str, p: serde_json::Value) -> Result<serde_json::Value, TransportError> {
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
        mmproj_path: None,
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
        json_schema: None,
        images: vec![]
    })
}

#[test]
fn a_full_conversation_streams_deltas_and_ends_with_a_result() {
    let mut t = start();
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
                json_schema: Some(schema),
                images: vec![]
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
}

#[test]
fn every_call_needs_a_hello_first() {
    let mut ok = start();
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
    call(&mut ok, "shutdown", json!({})).unwrap();
}

#[test]
fn typed_errors_and_panics_cross_the_wire() {
    let mut t = start();
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
    let mut t = start();
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
