//! One connection between two IMT processes after the handshake (ADR 0017 §1): either side calls the other
//! with [`Peer::call`] (one answer) or [`Peer::stream`] (items with credits, then one answer), and answers the
//! other side's calls with its [`Handler`].
//!
//! The API is synchronous so plugin code, importers and the engine (which are synchronous) use it directly:
//! two tasks on [`crate::runtime::shared`] read and write frames; calls the other side makes run on blocking
//! threads; a caller waits on a channel. Nothing here ever blocks the reader, so a handler may call back
//! into the other side (a plugin reading a host file while it imports) without deadlocking.
//!
//! Failure semantics: a typed error is [`IpcError::Remote`] (the other side is fine); a broken connection is
//! [`IpcError::Died`]; a deadline the callee ignored (it did not answer the cancel within [`CANCEL_GRACE`]) is
//! [`IpcError::Timeout`]; a frame that breaks the protocol (too large, garbled, more items than credits) closes
//! the connection, and every call on it fails with [`IpcError::Protocol`].

use crate::frame::{self, Frame, FrameError};
use crate::{IpcError, RemoteError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

/// Typed error codes every protocol shares (protocols add their own, positive or below -33000).
pub mod code {
    /// The callee does not implement this request.
    pub const UNIMPLEMENTED: i32 = -32601;
    /// The request did not decode or is not valid.
    pub const INVALID: i32 = -32602;
    /// The callee failed in a way it did not classify.
    pub const INTERNAL: i32 = -32603;
    /// The callee panicked while handling the call (it is still running; the panic counts as a crash).
    pub const PANIC: i32 = -32604;
    /// The call was cancelled.
    pub const CANCELLED: i32 = -32605;
    /// Too many calls are running on this connection.
    pub const BUSY: i32 = -32606;
}

/// How long a callee has to answer a cancel before the call counts as hung.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Items a stream may have in flight before the caller grants more (see [`Peer::stream`]).
pub const DEFAULT_CREDIT: u32 = 4;

/// A byte stream a peer runs on (a Unix socket, a named pipe, an in-memory duplex in tests).
pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Duplex for T {}

/// Answers the other side's calls. Runs on a blocking thread per call; must not panic (a panic is contained
/// and answered with [`code::PANIC`]).
pub trait Handler: Send + Sync + 'static {
    fn call(&self, ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RemoteError>;
}

/// A handler that refuses every call (for the side that only calls).
pub struct NoCalls;

impl Handler for NoCalls {
    fn call(&self, _ctx: &CallCtx, _body: &[u8]) -> Result<Vec<u8>, RemoteError> {
        Err(RemoteError::new(code::UNIMPLEMENTED, "this side answers no calls"))
    }
}

impl<F> Handler for F
where
    F: Fn(&CallCtx, &[u8]) -> Result<Vec<u8>, RemoteError> + Send + Sync + 'static,
{
    fn call(&self, ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RemoteError> {
        self(ctx, body)
    }
}

/// How a peer runs.
#[derive(Debug, Clone)]
pub struct PeerOptions {
    /// Names the other side in messages ("plugin \"x\"", "the AI engine").
    pub label: String,
    /// Largest frame accepted or sent (see [`frame::clamp_max`]).
    pub max_frame: usize,
    /// Calls of the other side handled at once; more are refused with [`code::BUSY`].
    pub max_concurrent_calls: usize,
    /// The protocol the host serves on this connection, checked against the child's in the handshake
    /// ([`frame::ProtocolVersion::accepts`]); [`frame::ProtocolVersion::UNVERSIONED`] by default.
    pub protocol: frame::ProtocolVersion,
}

impl PeerOptions {
    pub fn new(label: impl Into<String>) -> PeerOptions {
        PeerOptions {
            label: label.into(),
            max_frame: frame::DEFAULT_MAX_FRAME,
            max_concurrent_calls: 64,
            protocol: frame::ProtocolVersion::UNVERSIONED,
        }
    }
}

/// What a call's caller receives.
enum Event {
    Item(Vec<u8>),
    Reply(Vec<u8>),
    Fail(RemoteError),
    Closed(IpcError),
}

/// A call this side made and is waiting for.
struct Pending {
    tx: std_mpsc::Sender<Event>,
    /// Items the caller allowed so far (streams), and items received.
    granted: u64,
    received: u64,
}

/// A call of the other side this side is handling.
struct Served {
    state: Mutex<ServedState>,
    cv: Condvar,
}

struct ServedState {
    credit: u64,
    streams: bool,
    cancelled: bool,
}

enum Out {
    Frame(Vec<u8>),
    Shutdown,
}

struct Inner {
    opts: PeerOptions,
    max_frame: usize,
    out: mpsc::UnboundedSender<Out>,
    pending: Mutex<HashMap<u64, Pending>>,
    served: Mutex<HashMap<u64, Arc<Served>>>,
    next: AtomicU64,
    closed: Mutex<Option<IpcError>>,
    is_closed: AtomicBool,
    closed_cv: Condvar,
    closed_wait: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A live connection (cheap to clone). Dropping the last clone does not close it: call [`Peer::close`], or it
/// closes when the other side goes away.
#[derive(Clone)]
pub struct Peer {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer").field("label", &self.inner.opts.label).finish()
    }
}

impl Peer {
    /// Starts the reader and writer of a connection whose handshake is done, on `rt`.
    pub fn start(stream: impl Duplex, handler: Arc<dyn Handler>, opts: PeerOptions, rt: &tokio::runtime::Handle) -> Peer {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let max_frame = frame::clamp_max(opts.max_frame);
        let peer = Peer {
            inner: Arc::new(Inner {
                opts,
                max_frame,
                out: out_tx,
                pending: Mutex::new(HashMap::new()),
                served: Mutex::new(HashMap::new()),
                next: AtomicU64::new(1),
                closed: Mutex::new(None),
                is_closed: AtomicBool::new(false),
                closed_cv: Condvar::new(),
                closed_wait: Mutex::new(()),
            }),
        };
        let (read, write) = tokio::io::split(stream);
        rt.spawn(writer(peer.clone(), write, out_rx));
        rt.spawn(reader(peer.clone(), read, handler));
        peer
    }

    pub fn label(&self) -> &str {
        &self.inner.opts.label
    }

    /// The largest frame this connection carries.
    pub fn max_frame(&self) -> usize {
        self.inner.max_frame
    }

    /// Why the connection is closed, if it is.
    pub fn closed(&self) -> Option<IpcError> {
        lock(&self.inner.closed).clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed.load(Ordering::SeqCst)
    }

    /// Waits until the connection closes (at most `timeout`); returns whether it did.
    pub fn wait_closed(&self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut g = lock(&self.inner.closed_wait);
        while !self.is_closed() {
            let wait = match deadline {
                Some(d) => match d.checked_duration_since(Instant::now()) {
                    Some(w) => w.min(Duration::from_millis(200)),
                    None => return false,
                },
                None => Duration::from_millis(200),
            };
            g = self
                .inner
                .closed_cv
                .wait_timeout(g, wait)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        true
    }

    /// Closes the connection: pending calls fail, the other side sees the end of the stream.
    pub fn close(&self) {
        let _ = self.inner.out.send(Out::Shutdown);
        self.mark_closed(IpcError::Died(format!("the connection to {} was closed", self.label())));
    }

    fn mark_closed(&self, why: IpcError) {
        {
            let mut c = lock(&self.inner.closed);
            if c.is_some() {
                return;
            }
            *c = Some(why.clone());
        }
        self.inner.is_closed.store(true, Ordering::SeqCst);
        for (_, p) in lock(&self.inner.pending).drain() {
            let _ = p.tx.send(Event::Closed(why.clone()));
        }
        for (_, s) in lock(&self.inner.served).drain() {
            lock(&s.state).cancelled = true;
            s.cv.notify_all();
        }
        let _g = lock(&self.inner.closed_wait);
        self.inner.closed_cv.notify_all();
    }

    fn send(&self, f: &Frame) -> Result<(), IpcError> {
        if let Some(why) = self.closed() {
            return Err(why);
        }
        let bytes = frame::encode(f, self.inner.max_frame).map_err(|e| IpcError::Protocol(e.to_string()))?;
        self.inner.out.send(Out::Frame(bytes)).map_err(|_| {
            self.closed()
                .unwrap_or_else(|| IpcError::Died(format!("the connection to {} is gone", self.label())))
        })
    }

    fn begin(&self, body: Vec<u8>, credit: u32) -> Result<(u64, std_mpsc::Receiver<Event>), IpcError> {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std_mpsc::channel();
        lock(&self.inner.pending).insert(
            id,
            Pending {
                tx,
                granted: u64::from(credit),
                received: 0,
            },
        );
        if let Err(e) = self.send(&Frame::Call { id, credit, body }) {
            lock(&self.inner.pending).remove(&id);
            return Err(e);
        }
        Ok((id, rx))
    }

    fn cancel(&self, id: u64) {
        let _ = self.send(&Frame::Cancel { id });
    }

    /// Calls the other side and waits for its answer (at most `timeout`; `None` waits as long as the
    /// connection lives). After the timeout a cancel is sent; a callee that does not end the call within
    /// [`CANCEL_GRACE`] after that, or that ends it with a cancellation, makes this an [`IpcError::Timeout`].
    pub fn call(&self, body: Vec<u8>, timeout: Option<Duration>) -> Result<Vec<u8>, IpcError> {
        let (id, rx) = self.begin(body, 0)?;
        let first = match timeout {
            Some(t) => rx.recv_timeout(t),
            None => rx.recv().map_err(|_| std_mpsc::RecvTimeoutError::Disconnected),
        };
        let event = match first {
            Ok(e) => e,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                self.cancel(id);
                let late = rx.recv_timeout(CANCEL_GRACE);
                lock(&self.inner.pending).remove(&id);
                return match late {
                    Ok(Event::Reply(b)) => Ok(b),
                    Ok(Event::Closed(e)) => Err(e),
                    _ => Err(IpcError::Timeout),
                };
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return Err(self.gone()),
        };
        match event {
            Event::Reply(b) => Ok(b),
            Event::Fail(e) => Err(IpcError::Remote(e)),
            Event::Closed(e) => Err(e),
            Event::Item(_) => {
                let e = IpcError::Protocol(format!("{} sent an item for a call that does not stream", self.label()));
                self.mark_closed(e.clone());
                Err(e)
            }
        }
    }

    fn gone(&self) -> IpcError {
        self.closed()
            .unwrap_or_else(|| IpcError::Died(format!("the connection to {} is gone", self.label())))
    }

    /// [`Peer::call`] with typed request and reply (postcard).
    pub fn request<Req: Serialize, Rep: DeserializeOwned>(&self, req: &Req, timeout: Option<Duration>) -> Result<Rep, IpcError> {
        let body = crate::wire::encode(req).map_err(IpcError::Protocol)?;
        let reply = self.call(body, timeout)?;
        crate::wire::decode(&reply).map_err(|e| IpcError::Protocol(format!("{} answered with {e}", self.label())))
    }

    /// Starts a streamed call: the callee sends items (at most `credit` ahead of what this side took) and then
    /// one answer.
    pub fn stream(&self, body: Vec<u8>, credit: u32) -> Result<CallStream, IpcError> {
        let credit = credit.max(1);
        let (id, rx) = self.begin(body, credit)?;
        Ok(CallStream {
            peer: self.clone(),
            id,
            rx,
            window: credit,
            outstanding: credit,
            done: false,
        })
    }

    /// [`Peer::stream`] with a typed request.
    pub fn stream_request<Req: Serialize>(&self, req: &Req, credit: u32) -> Result<CallStream, IpcError> {
        let body = crate::wire::encode(req).map_err(IpcError::Protocol)?;
        self.stream(body, credit)
    }

    fn deliver(&self, id: u64, event: Event, last: bool) -> Result<(), IpcError> {
        let mut pending = lock(&self.inner.pending);
        let Some(p) = pending.get_mut(&id) else {
            // A late answer to a call that timed out or was cancelled.
            return Ok(());
        };
        if let Event::Item(_) = event {
            p.received += 1;
            if p.received > p.granted {
                return Err(IpcError::Protocol(format!(
                    "{} sent more items than it was granted ({} of {})",
                    self.label(),
                    p.received,
                    p.granted
                )));
            }
        }
        let _ = p.tx.send(event);
        if last {
            pending.remove(&id);
        }
        Ok(())
    }

    fn grant(&self, id: u64, n: u32) {
        if let Some(p) = lock(&self.inner.pending).get_mut(&id) {
            p.granted += u64::from(n);
        } else {
            return;
        }
        let _ = self.send(&Frame::Credit { id, n });
    }

    fn serve_call(&self, handler: &Arc<dyn Handler>, id: u64, credit: u32, body: Vec<u8>) {
        let served = {
            let mut s = lock(&self.inner.served);
            if s.len() >= self.inner.opts.max_concurrent_calls {
                drop(s);
                let _ = self.send(&Frame::Fail {
                    id,
                    error: RemoteError::new(code::BUSY, "too many calls are running; try again"),
                });
                return;
            }
            let served = Arc::new(Served {
                state: Mutex::new(ServedState {
                    credit: u64::from(credit),
                    streams: credit > 0,
                    cancelled: false,
                }),
                cv: Condvar::new(),
            });
            s.insert(id, served.clone());
            served
        };
        let (peer, handler) = (self.clone(), handler.clone());
        tokio::task::spawn_blocking(move || {
            let ctx = CallCtx {
                peer: peer.clone(),
                id,
                served,
            };
            let result = catch_unwind(AssertUnwindSafe(|| handler.call(&ctx, &body))).unwrap_or_else(|p| {
                Err(RemoteError::new(
                    code::PANIC,
                    format!("it panicked: {}", crate::panic_message(p.as_ref())),
                ))
            });
            lock(&peer.inner.served).remove(&id);
            let reply = match result {
                Ok(body) => Frame::Reply { id, body },
                Err(error) => Frame::Fail { id, error },
            };
            if let Err(IpcError::Protocol(e)) = peer.send(&reply) {
                // The answer is larger than a frame: say so instead.
                let _ = peer.send(&Frame::Fail {
                    id,
                    error: RemoteError::new(code::INTERNAL, format!("the answer could not be sent ({e})")),
                });
            }
        });
    }

    fn on_frame(&self, handler: &Arc<dyn Handler>, f: Frame) -> Result<(), IpcError> {
        match f {
            Frame::Call { id, credit, body } => {
                self.serve_call(handler, id, credit, body);
                Ok(())
            }
            Frame::Reply { id, body } => self.deliver(id, Event::Reply(body), true),
            Frame::Item { id, body } => self.deliver(id, Event::Item(body), false),
            Frame::Fail { id, error } => self.deliver(id, Event::Fail(error), true),
            Frame::Cancel { id } => {
                if let Some(s) = lock(&self.inner.served).get(&id) {
                    lock(&s.state).cancelled = true;
                    s.cv.notify_all();
                }
                Ok(())
            }
            Frame::Credit { id, n } => {
                if let Some(s) = lock(&self.inner.served).get(&id) {
                    let mut st = lock(&s.state);
                    st.credit = st.credit.saturating_add(u64::from(n));
                    s.cv.notify_all();
                }
                Ok(())
            }
            Frame::Hello { .. } | Frame::Welcome { .. } => Err(IpcError::Protocol(format!(
                "{} repeated the handshake on an open connection",
                self.label()
            ))),
        }
    }
}

async fn writer(peer: Peer, mut w: impl AsyncWrite + Unpin, mut rx: mpsc::UnboundedReceiver<Out>) {
    use tokio::io::AsyncWriteExt;
    while let Some(out) = rx.recv().await {
        match out {
            Out::Frame(bytes) => {
                if let Err(e) = w.write_all(&bytes).await {
                    peer.mark_closed(IpcError::Died(format!("the connection to {} broke ({e})", peer.label())));
                    return;
                }
                // Flush once the queue is empty, so bursts go out together.
                if rx.is_empty() && w.flush().await.is_err() {
                    peer.mark_closed(IpcError::Died(format!("the connection to {} broke", peer.label())));
                    return;
                }
            }
            Out::Shutdown => break,
        }
    }
    let _ = w.shutdown().await;
}

async fn reader(peer: Peer, mut r: impl AsyncRead + Unpin, handler: Arc<dyn Handler>) {
    loop {
        match frame::read_frame(&mut r, peer.inner.max_frame).await {
            Ok(f) => {
                if let Err(e) = peer.on_frame(&handler, f) {
                    peer.mark_closed(e);
                    let _ = peer.inner.out.send(Out::Shutdown);
                    return;
                }
            }
            Err(FrameError::Closed) => {
                peer.mark_closed(IpcError::Died(format!("{} closed the connection", peer.label())));
                return;
            }
            Err(FrameError::Io(e)) => {
                peer.mark_closed(IpcError::Died(format!("the connection to {} broke ({e})", peer.label())));
                return;
            }
            Err(e @ (FrameError::TooLarge { .. } | FrameError::Invalid(_))) => {
                peer.mark_closed(IpcError::Protocol(format!("{}: {e}", peer.label())));
                let _ = peer.inner.out.send(Out::Shutdown);
                return;
            }
        }
    }
}

/// What a streamed call yields.
#[derive(Debug)]
pub enum StreamEvent {
    /// One item.
    Item(Vec<u8>),
    /// The call's answer; the stream is complete.
    Done(Vec<u8>),
    /// Nothing arrived within the timeout given to [`CallStream::next`]; the call is still running.
    Idle,
}

/// The caller's side of a streamed call. Dropping it before the end cancels the call.
pub struct CallStream {
    peer: Peer,
    id: u64,
    rx: std_mpsc::Receiver<Event>,
    window: u32,
    /// Items granted but not taken yet.
    outstanding: u32,
    done: bool,
}

impl CallStream {
    /// The next item or the answer, waiting at most `timeout` (`None`: as long as the connection lives).
    pub fn next(&mut self, timeout: Option<Duration>) -> Result<StreamEvent, IpcError> {
        if self.done {
            return Err(IpcError::Protocol("the stream is already complete".into()));
        }
        let got = match timeout {
            Some(t) => match self.rx.recv_timeout(t) {
                Ok(e) => e,
                Err(std_mpsc::RecvTimeoutError::Timeout) => return Ok(StreamEvent::Idle),
                Err(std_mpsc::RecvTimeoutError::Disconnected) => return Err(self.peer.gone()),
            },
            None => self.rx.recv().map_err(|_| self.peer.gone())?,
        };
        match got {
            Event::Item(b) => {
                self.outstanding = self.outstanding.saturating_sub(1);
                if self.outstanding <= self.window / 2 {
                    let n = self.window - self.outstanding;
                    self.outstanding = self.window;
                    self.peer.grant(self.id, n);
                }
                Ok(StreamEvent::Item(b))
            }
            Event::Reply(b) => {
                self.done = true;
                Ok(StreamEvent::Done(b))
            }
            Event::Fail(e) => {
                self.done = true;
                Err(IpcError::Remote(e))
            }
            Event::Closed(e) => {
                self.done = true;
                Err(e)
            }
        }
    }

    /// Asks the callee to stop; [`CallStream::next`] then yields its answer (usually a cancellation error).
    pub fn cancel(&mut self) {
        if !self.done {
            self.peer.cancel(self.id);
        }
    }

    /// Cancels and waits up to [`CANCEL_GRACE`] for the callee to end the call; `Err(Timeout)` if it did not.
    pub fn cancel_and_wait(&mut self) -> Result<(), IpcError> {
        self.cancel();
        let deadline = Instant::now() + CANCEL_GRACE;
        while !self.done {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(IpcError::Timeout);
            }
            match self.next(Some(left)) {
                Ok(StreamEvent::Idle) => return Err(IpcError::Timeout),
                Ok(_) | Err(IpcError::Remote(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl Drop for CallStream {
    fn drop(&mut self) {
        if !self.done {
            self.peer.cancel(self.id);
            lock(&self.peer.inner.pending).remove(&self.id);
        }
    }
}

/// What a [`Handler`] gets for one call.
pub struct CallCtx {
    peer: Peer,
    id: u64,
    served: Arc<Served>,
}

impl CallCtx {
    /// The connection, to call back into the caller's side (e.g. to read a host file).
    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    /// Whether the caller cancelled the call (or the connection closed).
    pub fn is_cancelled(&self) -> bool {
        lock(&self.served.state).cancelled
    }

    /// A probe of [`CallCtx::is_cancelled`] that outlives the borrow of the context, for work that runs on other
    /// threads (e.g. a watcher that interrupts a database statement when the caller cancels).
    pub fn cancel_probe(&self) -> CancelProbe {
        CancelProbe(self.served.clone())
    }

    /// The call as cancelled, for handlers that check [`CallCtx::is_cancelled`].
    pub fn cancelled_error() -> RemoteError {
        RemoteError::new(code::CANCELLED, "the call was cancelled")
    }

    /// Sends one item of a streamed answer, waiting until the caller has granted credit for it. Fails when the
    /// call was cancelled, the connection closed, or the caller did not ask for a stream.
    pub fn send_item(&self, body: Vec<u8>) -> Result<(), RemoteError> {
        let mut st = lock(&self.served.state);
        if !st.streams {
            return Err(RemoteError::new(code::INVALID, "this call does not stream"));
        }
        loop {
            if st.cancelled {
                return Err(Self::cancelled_error());
            }
            if st.credit > 0 {
                st.credit -= 1;
                break;
            }
            st = self
                .served
                .cv
                .wait_timeout(st, Duration::from_millis(100))
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        drop(st);
        self.peer
            .send(&Frame::Item { id: self.id, body })
            .map_err(|e| RemoteError::new(code::INTERNAL, e.to_string()))
    }

    /// [`CallCtx::send_item`] with a typed item.
    pub fn send<T: Serialize>(&self, item: &T) -> Result<(), RemoteError> {
        let body = crate::wire::encode(item).map_err(|e| RemoteError::new(code::INTERNAL, e))?;
        self.send_item(body)
    }
}

/// Whether a served call was cancelled (or its connection closed); see [`CallCtx::cancel_probe`].
#[derive(Clone)]
pub struct CancelProbe(Arc<Served>);

impl CancelProbe {
    pub fn is_cancelled(&self) -> bool {
        lock(&self.0.state).cancelled
    }
}

/// Decodes a typed request for a handler; a body that does not decode is [`code::INVALID`].
pub fn decode_request<T: DeserializeOwned>(body: &[u8]) -> Result<T, RemoteError> {
    crate::wire::decode(body).map_err(|e| RemoteError::new(code::INVALID, e))
}

/// Encodes a typed reply for a handler.
pub fn encode_reply<T: Serialize>(reply: &T) -> Result<Vec<u8>, RemoteError> {
    crate::wire::encode(reply).map_err(|e| RemoteError::new(code::INTERNAL, e))
}

#[cfg(test)]
mod tests;
