use super::*;
use std::sync::atomic::AtomicUsize;

fn rt() -> &'static tokio::runtime::Runtime {
    crate::runtime::shared().unwrap()
}

/// Two peers connected in memory.
fn pair(a: Arc<dyn Handler>, b: Arc<dyn Handler>) -> (Peer, Peer) {
    let (x, y) = tokio::io::duplex(1 << 16);
    let pa = Peer::start(x, a, PeerOptions::new("side a"), rt().handle());
    let pb = Peer::start(y, b, PeerOptions::new("side b"), rt().handle());
    (pa, pb)
}

/// A peer and the raw other end of its stream.
fn raw(handler: Arc<dyn Handler>, opts: PeerOptions) -> (Peer, tokio::io::DuplexStream) {
    let (x, y) = tokio::io::duplex(1 << 16);
    (Peer::start(x, handler, opts, rt().handle()), y)
}

fn echo() -> Arc<dyn Handler> {
    Arc::new(|_: &CallCtx, body: &[u8]| -> Result<Vec<u8>, RemoteError> {
        match body {
            b"fail" => Err(RemoteError::new(7, "typed failure")),
            b"panic" => panic!("handler panicked"),
            other => Ok(other.to_vec()),
        }
    })
}

const T: Option<Duration> = Some(Duration::from_secs(10));

#[test]
fn calls_go_both_ways_with_typed_errors_and_contained_panics() {
    let (a, b) = pair(echo(), echo());
    assert_eq!(a.call(b"hello".to_vec(), T).unwrap(), b"hello");
    assert_eq!(b.call(b"back".to_vec(), T).unwrap(), b"back");
    assert_eq!(
        a.call(b"fail".to_vec(), T).unwrap_err(),
        IpcError::Remote(RemoteError::new(7, "typed failure"))
    );
    match a.call(b"panic".to_vec(), T).unwrap_err() {
        IpcError::Remote(e) => assert!(e.code == code::PANIC && e.message.contains("handler panicked"), "{e:?}"),
        other => panic!("{other:?}"),
    }
    // Still usable after a panic.
    assert_eq!(a.call(b"again".to_vec(), T).unwrap(), b"again");
    // Typed helpers.
    let got: String = a.request(&"typed".to_string(), T).unwrap();
    assert_eq!(got, "typed");
    assert_eq!(a.label(), "side a");
    assert_eq!(a.max_frame(), frame::DEFAULT_MAX_FRAME);
    assert!(format!("{a:?}").contains("side a"));
    // The side that answers nothing.
    let (c, _d) = pair(Arc::new(NoCalls), Arc::new(NoCalls));
    assert!(matches!(c.call(Vec::new(), T), Err(IpcError::Remote(e)) if e.code == code::UNIMPLEMENTED));
}

#[test]
fn streams_respect_credits_and_end_with_the_reply() {
    let sent = Arc::new(AtomicUsize::new(0));
    let s2 = sent.clone();
    let producer: Arc<dyn Handler> = Arc::new(move |ctx: &CallCtx, body: &[u8]| -> Result<Vec<u8>, RemoteError> {
        let n: u32 = decode_request(body)?;
        for i in 0..n {
            ctx.send(&i)?;
            s2.fetch_add(1, Ordering::SeqCst);
        }
        encode_reply(&format!("{n} sent"))
    });
    let (a, _b) = pair(Arc::new(NoCalls), producer);
    let mut s = a.stream_request(&100u32, 4).unwrap();
    let mut got = Vec::new();
    loop {
        match s.next(T).unwrap() {
            StreamEvent::Item(b) => {
                // The producer is never more than the window ahead of what was taken.
                std::thread::sleep(Duration::from_millis(1));
                assert!(
                    sent.load(Ordering::SeqCst) <= got.len() + 1 + 4,
                    "{} sent, {} taken",
                    sent.load(Ordering::SeqCst),
                    got.len()
                );
                got.push(crate::wire::decode::<u32>(&b).unwrap());
            }
            StreamEvent::Done(b) => {
                assert_eq!(crate::wire::decode::<String>(&b).unwrap(), "100 sent");
                break;
            }
            StreamEvent::Idle => panic!("idle"),
        }
    }
    assert_eq!(got, (0..100).collect::<Vec<_>>());
    assert!(s.is_done());
    assert!(s.next(T).is_err(), "nothing after the end");
}

#[test]
fn a_slow_consumer_holds_the_producer_back() {
    let sent = Arc::new(AtomicUsize::new(0));
    let s2 = sent.clone();
    let producer: Arc<dyn Handler> = Arc::new(move |ctx: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        for _ in 0..1000 {
            ctx.send_item(vec![0; 10])?;
            s2.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Vec::new())
    });
    let (a, _b) = pair(Arc::new(NoCalls), producer);
    let mut s = a.stream(Vec::new(), 8).unwrap();
    assert!(matches!(s.next(T).unwrap(), StreamEvent::Item(_)));
    std::thread::sleep(Duration::from_millis(200));
    // One taken, so at most 8 more were granted (8 initially, more only after taking half the window).
    assert!(sent.load(Ordering::SeqCst) <= 8, "{}", sent.load(Ordering::SeqCst));
}

#[test]
fn dropping_a_stream_cancels_the_call() {
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let s2 = saw_cancel.clone();
    let producer: Arc<dyn Handler> = Arc::new(move |ctx: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        loop {
            if let Err(e) = ctx.send_item(vec![1]) {
                assert_eq!(e.code, code::CANCELLED);
                s2.store(true, Ordering::SeqCst);
                return Err(e);
            }
        }
    });
    let (a, _b) = pair(Arc::new(NoCalls), producer);
    let mut s = a.stream(Vec::new(), 2).unwrap();
    assert!(matches!(s.next(T).unwrap(), StreamEvent::Item(_)));
    drop(s);
    let started = Instant::now();
    while !saw_cancel.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_cancel.load(Ordering::SeqCst));
    // An explicit cancel that the callee honours ends the stream.
    let mut s = a.stream(Vec::new(), 2).unwrap();
    assert!(s.cancel_and_wait().is_ok());
    assert!(s.is_done());
}

#[test]
fn a_callee_that_ignores_the_deadline_and_the_cancel_is_a_timeout() {
    let slow: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, body: &[u8]| -> Result<Vec<u8>, RemoteError> {
        if body == b"polite" {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            return Err(CallCtx::cancelled_error());
        }
        std::thread::sleep(Duration::from_secs(4));
        Ok(Vec::new())
    });
    let (a, _b) = pair(Arc::new(NoCalls), slow);
    let started = Instant::now();
    assert_eq!(
        a.call(b"rude".to_vec(), Some(Duration::from_millis(50))).unwrap_err(),
        IpcError::Timeout
    );
    assert!(started.elapsed() >= CANCEL_GRACE);
    // One that answers the cancel is still a timeout for the caller, but quick.
    let started = Instant::now();
    assert_eq!(
        a.call(b"polite".to_vec(), Some(Duration::from_millis(50))).unwrap_err(),
        IpcError::Timeout
    );
    assert!(started.elapsed() < CANCEL_GRACE);
    // An idle stream says so.
    let mut s = a.stream(b"polite".to_vec(), 1).unwrap();
    assert!(matches!(s.next(Some(Duration::from_millis(20))).unwrap(), StreamEvent::Idle));
}

#[test]
fn handlers_may_call_back_while_they_answer() {
    let host: Arc<dyn Handler> = Arc::new(|_: &CallCtx, body: &[u8]| -> Result<Vec<u8>, RemoteError> { Ok([b"host:", body].concat()) });
    let child: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, body: &[u8]| -> Result<Vec<u8>, RemoteError> {
        // e.g. a plugin reading a host file while it imports.
        let back = ctx
            .peer()
            .call(body.to_vec(), T)
            .map_err(|e| RemoteError::new(code::INTERNAL, e.to_string()))?;
        Ok([b"child got ", back.as_slice()].concat())
    });
    let (h, _c) = pair(host, child);
    assert_eq!(h.call(b"x".to_vec(), T).unwrap(), b"child got host:x");
}

#[test]
fn too_many_concurrent_calls_are_refused_as_busy() {
    let gate = Arc::new(AtomicBool::new(false));
    let g2 = gate.clone();
    let blocking: Arc<dyn Handler> = Arc::new(move |_: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        while !g2.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(Vec::new())
    });
    let (x, y) = tokio::io::duplex(1 << 16);
    let a = Peer::start(x, Arc::new(NoCalls), PeerOptions::new("a"), rt().handle());
    let mut opts = PeerOptions::new("b");
    opts.max_concurrent_calls = 1;
    let _b = Peer::start(y, blocking, opts, rt().handle());
    let a2 = a.clone();
    let first = std::thread::spawn(move || a2.call(Vec::new(), T));
    std::thread::sleep(Duration::from_millis(100));
    assert!(matches!(a.call(Vec::new(), T), Err(IpcError::Remote(e)) if e.code == code::BUSY));
    gate.store(true, Ordering::SeqCst);
    assert!(first.join().unwrap().is_ok());
}

#[test]
fn closing_fails_pending_calls_on_both_sides() {
    let slow: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        while !ctx.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(CallCtx::cancelled_error())
    });
    let (a, b) = pair(Arc::new(NoCalls), slow);
    let a2 = a.clone();
    let pending = std::thread::spawn(move || a2.call(Vec::new(), T));
    std::thread::sleep(Duration::from_millis(100));
    b.close();
    assert!(matches!(pending.join().unwrap(), Err(IpcError::Died(_))));
    assert!(a.wait_closed(Some(Duration::from_secs(5))));
    assert!(a.is_closed() && b.is_closed());
    assert!(matches!(a.closed(), Some(IpcError::Died(_))));
    assert!(matches!(a.call(Vec::new(), T), Err(IpcError::Died(_))));
    assert!(matches!(a.stream(Vec::new(), 1), Err(IpcError::Died(_))));
    // Waiting on an open connection times out.
    let (c, _d) = pair(Arc::new(NoCalls), Arc::new(NoCalls));
    assert!(!c.wait_closed(Some(Duration::from_millis(20))));
}

fn write_raw(io: &mut tokio::io::DuplexStream, bytes: &[u8]) {
    use tokio::io::AsyncWriteExt;
    rt().block_on(async { io.write_all(bytes).await.unwrap() });
}

fn write_frame_raw(io: &mut tokio::io::DuplexStream, f: &Frame) {
    write_raw(io, &frame::encode(f, frame::HARD_MAX_FRAME).unwrap());
}

fn read_frame_raw(io: &mut tokio::io::DuplexStream) -> Frame {
    rt().block_on(async { tokio::time::timeout(Duration::from_secs(5), frame::read_frame(io, frame::HARD_MAX_FRAME)).await })
        .unwrap()
        .unwrap()
}

#[test]
fn an_oversized_frame_closes_the_connection_as_a_protocol_failure() {
    let (p, mut io) = raw(Arc::new(NoCalls), PeerOptions::new("the plugin"));
    let p2 = p.clone();
    let pending = std::thread::spawn(move || p2.call(Vec::new(), T));
    let _call = read_frame_raw(&mut io);
    write_raw(&mut io, &(100u32 << 20).to_le_bytes());
    match pending.join().unwrap() {
        Err(IpcError::Protocol(m)) => assert!(m.contains("exceeds the limit"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(p.wait_closed(Some(Duration::from_secs(5))));
}

#[test]
fn garbage_closes_the_connection() {
    let (p, mut io) = raw(Arc::new(NoCalls), PeerOptions::new("the plugin"));
    write_raw(&mut io, &[5, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff]);
    assert!(p.wait_closed(Some(Duration::from_secs(5))));
    assert!(matches!(p.closed(), Some(IpcError::Protocol(_))));
}

#[test]
fn a_repeated_handshake_or_items_beyond_credit_break_the_protocol() {
    let (p, mut io) = raw(Arc::new(NoCalls), PeerOptions::new("the plugin"));
    write_frame_raw(
        &mut io,
        &Frame::Welcome {
            version: 1,
            protocol: Default::default(),
        },
    );
    assert!(p.wait_closed(Some(Duration::from_secs(5))));
    assert!(matches!(p.closed(), Some(IpcError::Protocol(m)) if m.contains("handshake")));

    let (p, mut io) = raw(Arc::new(NoCalls), PeerOptions::new("the plugin"));
    // Nothing is taken, so no credit is granted beyond the first item.
    let mut s = p.stream(Vec::new(), 1).unwrap();
    let id = read_frame_raw(&mut io).call_id().unwrap();
    write_frame_raw(&mut io, &Frame::Item { id, body: vec![1] });
    write_frame_raw(&mut io, &Frame::Item { id, body: vec![2] });
    assert!(p.wait_closed(Some(Duration::from_secs(5))));
    assert!(matches!(s.next(T).unwrap(), StreamEvent::Item(_)));
    assert!(matches!(p.closed(), Some(IpcError::Protocol(m)) if m.contains("more items")));

    // An item for a call that does not stream.
    let (p, mut io) = raw(Arc::new(NoCalls), PeerOptions::new("the plugin"));
    let p2 = p.clone();
    let pending = std::thread::spawn(move || p2.call(Vec::new(), T));
    let id = read_frame_raw(&mut io).call_id().unwrap();
    write_frame_raw(&mut io, &Frame::Item { id, body: vec![1] });
    assert!(matches!(pending.join().unwrap(), Err(IpcError::Protocol(_))));
}

#[test]
fn credits_and_cancels_from_the_other_side_are_honoured() {
    // A raw caller: calls a producer with credit 1, grants one more, then cancels.
    let producer: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        let mut n = 0u8;
        loop {
            ctx.send_item(vec![n])?;
            n += 1;
        }
    });
    let (_p, mut io) = raw(producer, PeerOptions::new("producer"));
    write_frame_raw(
        &mut io,
        &Frame::Call {
            id: 9,
            credit: 1,
            body: Vec::new(),
        },
    );
    assert_eq!(read_frame_raw(&mut io), Frame::Item { id: 9, body: vec![0] });
    write_frame_raw(&mut io, &Frame::Credit { id: 9, n: 1 });
    assert_eq!(read_frame_raw(&mut io), Frame::Item { id: 9, body: vec![1] });
    write_frame_raw(&mut io, &Frame::Cancel { id: 9 });
    match read_frame_raw(&mut io) {
        Frame::Fail { id: 9, error } => assert_eq!(error.code, code::CANCELLED),
        other => panic!("{other:?}"),
    }
    // A unary call cannot stream.
    let streaming: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> {
        ctx.send_item(Vec::new())?;
        Ok(Vec::new())
    });
    let (a, _b) = pair(Arc::new(NoCalls), streaming);
    assert!(matches!(a.call(Vec::new(), T), Err(IpcError::Remote(e)) if e.code == code::INVALID));
}

#[test]
fn an_answer_too_large_for_a_frame_is_a_typed_error() {
    let big: Arc<dyn Handler> = Arc::new(|_: &CallCtx, _: &[u8]| -> Result<Vec<u8>, RemoteError> { Ok(vec![0; 4096]) });
    let (x, y) = tokio::io::duplex(1 << 16);
    let a = Peer::start(x, Arc::new(NoCalls), PeerOptions::new("a"), rt().handle());
    let mut opts = PeerOptions::new("b");
    opts.max_frame = 1024;
    let _b = Peer::start(y, big, opts, rt().handle());
    assert!(
        matches!(a.call(Vec::new(), T), Err(IpcError::Remote(e)) if e.code == code::INTERNAL && e.message.contains("could not be sent"))
    );
}
