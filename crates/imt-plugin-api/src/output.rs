//! Output files of a tool (ADR 0017 §7 and its amendment of 2026-10-22): a tool writes a capture through
//! [`ToolCtx::outputs`](crate::tools::ToolCtx::outputs) instead of at a path.
//!
//! In a plugin's elevated process the bytes are streamed to the unelevated host ([`HostRequest::SaveOutput`],
//! answered while the host pulls [`PluginRequest::PullOutput`](crate::protocol::PluginRequest::PullOutput) items
//! with credits), and the *host* writes them into the data folder it chose, so the elevated process never writes
//! at a path. [`FolderOutputs`] writes the same files directly (the host's side, tests and hosts without an
//! elevated process).
//!
//! An [`OutputFile`] dropped without [`OutputFile::finish`] is discarded: nothing is left at its name.

pub use crate::protocol::SavedOutput;
use crate::protocol::{check_output_name, HostRequest, OutputEnd, MAX_OUTPUT_BYTES, OUTPUT_CHUNK, OUTPUT_ROOT};
use anyhow::{anyhow, bail, Context, Result};
use imt_ipc::peer::{code, encode_reply, CallCtx};
use imt_ipc::{IpcError, Peer, RemoteError};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Pieces a stream buffers between the tool and the connection; with the host's credits this bounds the bytes
/// in flight, and a tool writing faster than the host takes them waits.
const QUEUE: usize = 2;
/// How long the host may take to accept a stream.
const ACCEPT_WAIT: Duration = Duration::from_secs(30);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// An output file being written. `Write` errors say why the stream stopped (e.g. the host cut it at its size
/// limit).
pub trait OutputFile: Write + Send {
    /// Completes the file and returns what was saved. Dropping it without `finish` discards it.
    fn finish(self: Box<Self>) -> Result<SavedOutput>;
}

/// Where a tool's output files go.
pub trait Outputs: Send + Sync {
    /// A new file `<data>/captures/<folder>/<name>` of at most `max_bytes` (and [`MAX_OUTPUT_BYTES`]). Fails when
    /// a name is not one plain component ([`check_output_name`]) or the file exists.
    fn create(&self, folder: &str, name: &str, max_bytes: u64) -> Result<Box<dyn OutputFile>>;
}

// ---- writing files directly ----

/// Output files written directly into `<root>/captures/<folder>/<name>`: the names are checked, a file is never
/// replaced, the bytes go to a hidden partial file renamed at the end, and a file that fails or is dropped is
/// removed.
pub struct FolderOutputs(pub PathBuf);

impl Outputs for FolderOutputs {
    fn create(&self, folder: &str, name: &str, max_bytes: u64) -> Result<Box<dyn OutputFile>> {
        Ok(Box::new(LocalFile::create(&self.0, folder, name, max_bytes)?))
    }
}

/// A file [`FolderOutputs`] writes.
pub struct LocalFile {
    file: Option<File>,
    part: PathBuf,
    path: PathBuf,
    name: String,
    written: u64,
    max: u64,
}

impl LocalFile {
    /// Starts `<root>/captures/<folder>/<name>`; see [`FolderOutputs`].
    pub fn create(root: &Path, folder: &str, name: &str, max_bytes: u64) -> Result<LocalFile> {
        check_output_name("folder", folder).map_err(|e| anyhow!(e))?;
        check_output_name("file name", name).map_err(|e| anyhow!(e))?;
        let dir = root.join(OUTPUT_ROOT).join(folder);
        std::fs::create_dir_all(&dir).with_context(|| format!("cannot create the folder {}", dir.display()))?;
        // The folder must still be inside the root once links are resolved.
        let (canon_root, canon_dir) = (root.canonicalize()?, dir.canonicalize()?);
        if !canon_dir.starts_with(&canon_root) {
            bail!("the output folder {} leads outside {}", dir.display(), root.display());
        }
        let path = canon_dir.join(name);
        if path.symlink_metadata().is_ok() {
            bail!("{} already exists; IMT does not replace it", path.display());
        }
        let part = canon_dir.join(format!(".{name}.part"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .with_context(|| format!("cannot create {}", part.display()))?;
        Ok(LocalFile {
            file: Some(file),
            part,
            path,
            name: name.to_string(),
            written: 0,
            max: max_bytes.min(MAX_OUTPUT_BYTES),
        })
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The limit of this file in bytes.
    pub fn limit(&self) -> u64 {
        self.max
    }

    fn complete(&mut self) -> Result<SavedOutput> {
        let file = self.file.take().ok_or_else(|| anyhow!("the output file was already finished"))?;
        file.sync_all().context("cannot write the output file to disk")?;
        drop(file);
        if self.path.symlink_metadata().is_ok() {
            bail!("{} already exists; IMT does not replace it", self.path.display());
        }
        std::fs::rename(&self.part, &self.path).with_context(|| format!("cannot save {}", self.path.display()))?;
        Ok(SavedOutput {
            name: self.name.clone(),
            bytes: self.written,
        })
    }
}

impl Write for LocalFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("the output file was already finished"))?;
        if self.written + buf.len() as u64 > self.max {
            return Err(io::Error::other(format!(
                "the output exceeds its limit of {} bytes and was cut off",
                self.max
            )));
        }
        file.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), |f| f.flush())
    }
}

impl OutputFile for LocalFile {
    fn finish(mut self: Box<Self>) -> Result<SavedOutput> {
        self.complete()
    }
}

impl Drop for LocalFile {
    fn drop(&mut self) {
        // Not finished (or finishing failed): nothing is left behind.
        if self.part.exists() {
            self.file = None;
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

// ---- streaming to the host (the elevated process) ----

/// A stream waiting for the host's [`PluginRequest::PullOutput`](crate::protocol::PluginRequest::PullOutput).
enum Slot {
    Waiting(Receiver<Option<Vec<u8>>>),
    Pulled,
}

#[derive(Default)]
struct Streams {
    slots: Mutex<HashMap<u64, Slot>>,
    changed: Condvar,
    next: AtomicU64,
}

fn streams() -> &'static Streams {
    static S: OnceLock<Streams> = OnceLock::new();
    S.get_or_init(Streams::default)
}

/// Output files of the elevated process, saved by the host.
pub(crate) struct HostOutputs {
    pub(crate) peer: Peer,
    pub(crate) tool: String,
}

impl Outputs for HostOutputs {
    fn create(&self, folder: &str, name: &str, max_bytes: u64) -> Result<Box<dyn OutputFile>> {
        let s = streams();
        let id = s.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = sync_channel(QUEUE);
        lock(&s.slots).insert(id, Slot::Waiting(rx));
        let req = HostRequest::SaveOutput {
            tool: self.tool.clone(),
            stream: id,
            folder: folder.into(),
            name: name.into(),
            max_bytes,
        };
        let peer = self.peer.clone();
        let label = format!("{folder}/{name}");
        let thread = std::thread::Builder::new()
            .name("imt-plugin-output".into())
            .spawn(move || {
                // The host answers when the stream ended: as long as the connection lives.
                let r: Result<Result<SavedOutput, String>, IpcError> = peer.request(&req, None);
                let s = streams();
                lock(&s.slots).remove(&id);
                s.changed.notify_all();
                match r {
                    Ok(r) => r,
                    Err(e) => Err(format!("saving {label} failed: {e}")),
                }
            })
            .context("cannot start the output stream")?;
        // Wait until the host accepted the stream (pulls it) or refused it (answered).
        let deadline = Instant::now() + ACCEPT_WAIT;
        let mut g = lock(&s.slots);
        loop {
            match g.get(&id) {
                Some(Slot::Pulled) => break,
                None => {
                    drop(g);
                    let why = thread.join().unwrap_or_else(|_| Err("the output stream failed".into()));
                    return Err(anyhow!(why.err().unwrap_or_else(|| "the host ended the output at once".into())));
                }
                Some(Slot::Waiting(_)) if Instant::now() >= deadline => {
                    g.remove(&id);
                    return Err(anyhow!("IMT did not accept the output {name} in time"));
                }
                Some(Slot::Waiting(_)) => {
                    g = s
                        .changed
                        .wait_timeout(g, Duration::from_millis(100))
                        .map(|(g, _)| g)
                        .unwrap_or_else(|p| p.into_inner().0);
                }
            }
        }
        drop(g);
        Ok(Box::new(StreamFile {
            tx: Some(tx),
            buf: Vec::with_capacity(OUTPUT_CHUNK),
            thread: Some(thread),
            failed: None,
        }))
    }
}

/// An output file streamed to the host.
struct StreamFile {
    tx: Option<SyncSender<Option<Vec<u8>>>>,
    buf: Vec<u8>,
    thread: Option<JoinHandle<Result<SavedOutput, String>>>,
    /// Why the stream stopped, once it did.
    failed: Option<String>,
}

impl StreamFile {
    /// The host's answer (waits for it).
    fn answer(&mut self) -> Result<SavedOutput, String> {
        match self.thread.take() {
            Some(t) => t.join().unwrap_or_else(|_| Err("the output stream failed".into())),
            None => Err(self.failed.clone().unwrap_or_else(|| "the output was already finished".into())),
        }
    }

    fn fail(&mut self) -> io::Error {
        self.tx = None;
        let why = match self.answer() {
            Err(e) => e,
            Ok(_) => "the host ended the output early".into(),
        };
        self.failed = Some(why.clone());
        io::Error::other(why)
    }

    fn send(&mut self, piece: Option<Vec<u8>>) -> io::Result<()> {
        if let Some(why) = &self.failed {
            return Err(io::Error::other(why.clone()));
        }
        let Some(tx) = &self.tx else {
            return Err(io::Error::other("the output was already finished"));
        };
        // Blocks while the queue is full: the host has not granted credit for more.
        if tx.send(piece).is_err() {
            return Err(self.fail());
        }
        Ok(())
    }
}

impl Write for StreamFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let room = OUTPUT_CHUNK - self.buf.len();
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() >= OUTPUT_CHUNK {
            let piece = std::mem::replace(&mut self.buf, Vec::with_capacity(OUTPUT_CHUNK));
            self.send(Some(piece))?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let piece = std::mem::take(&mut self.buf);
            self.send(Some(piece))?;
        }
        Ok(())
    }
}

impl OutputFile for StreamFile {
    fn finish(mut self: Box<Self>) -> Result<SavedOutput> {
        self.flush()?;
        self.send(None)?;
        self.tx = None;
        self.answer().map_err(|e| anyhow!(e))
    }
}

impl Drop for StreamFile {
    fn drop(&mut self) {
        // Closing the queue without the end marker discards the stream: the host removes the partial file.
        self.tx = None;
    }
}

/// Answers the host's [`PluginRequest::PullOutput`](crate::protocol::PluginRequest::PullOutput): sends the pieces
/// of stream `id` as the host grants credit, then its size.
pub(crate) fn serve_pull(ctx: &CallCtx, id: u64) -> Result<Vec<u8>, RemoteError> {
    let s = streams();
    let rx = {
        let mut g = lock(&s.slots);
        match g.insert(id, Slot::Pulled) {
            Some(Slot::Waiting(rx)) => rx,
            other => {
                if other.is_none() {
                    g.remove(&id);
                }
                return Err(RemoteError::new(code::INVALID, format!("there is no output stream {id} to pull")));
            }
        }
    };
    s.changed.notify_all();
    let mut bytes = 0u64;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(piece)) => {
                bytes += piece.len() as u64;
                // Waits for the host's credit; fails when the host cut the stream off.
                ctx.send_item(piece)?;
            }
            Ok(None) => return encode_reply(&OutputEnd { bytes }),
            Err(RecvTimeoutError::Timeout) if ctx.is_cancelled() => return Err(CallCtx::cancelled_error()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(RemoteError::new(
                    code::INTERNAL,
                    "the plugin discarded the output before finishing it",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PluginRequest, OUTPUT_CREDIT};
    use imt_ipc::peer::{decode_request, Handler, StreamEvent};
    use imt_ipc::PeerOptions;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::sync::Arc;

    fn failure(r: Result<Box<dyn OutputFile>>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    /// A plugin-side peer serving `PullOutput`, connected in memory to a host-side peer answering with `host`.
    fn pair(host: Arc<dyn Handler>) -> (Peer, Peer) {
        let rt = imt_ipc::runtime::shared().unwrap();
        let (a, b) = tokio::io::duplex(1 << 20);
        let plugin: Arc<dyn Handler> = Arc::new(|ctx: &CallCtx, body: &[u8]| match decode_request(body)? {
            PluginRequest::PullOutput { stream } => serve_pull(ctx, stream),
            _ => Err(RemoteError::new(code::UNIMPLEMENTED, "no")),
        });
        let p = Peer::start(a, plugin, PeerOptions::new("plugin"), rt.handle());
        let h = Peer::start(b, host, PeerOptions::new("IMT"), rt.handle());
        (p, h)
    }

    #[test]
    fn a_stream_waits_for_the_hosts_credit_instead_of_buffering() {
        // The host accepts the stream, then takes nothing until `go`.
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = Mutex::new(go_rx);
        let host: Arc<dyn Handler> = Arc::new(move |ctx: &CallCtx, body: &[u8]| {
            let HostRequest::SaveOutput { stream, name, .. } = decode_request(body)? else {
                return Err(RemoteError::new(code::UNIMPLEMENTED, "no"));
            };
            let mut s = ctx
                .peer()
                .stream_request(&PluginRequest::PullOutput { stream }, OUTPUT_CREDIT)
                .map_err(|e| RemoteError::new(code::INTERNAL, e.to_string()))?;
            let _ = lock(&go_rx).recv();
            let mut bytes = 0u64;
            loop {
                match s.next(None).map_err(|e| RemoteError::new(code::INTERNAL, e.to_string()))? {
                    StreamEvent::Item(b) => {
                        assert!(b.len() <= OUTPUT_CHUNK);
                        bytes += b.len() as u64;
                    }
                    StreamEvent::Done(b) => {
                        let end: OutputEnd = imt_ipc::wire::decode(&b).unwrap();
                        assert_eq!(end.bytes, bytes);
                        return encode_reply(&Ok::<_, String>(SavedOutput { name, bytes }));
                    }
                    StreamEvent::Idle => {}
                }
            }
        });
        let (plugin, _host_peer) = pair(host);
        let outputs = HostOutputs {
            peer: plugin,
            tool: "t".into(),
        };
        let mut f = outputs.create("f", "n.bin", u64::MAX).unwrap();
        let pieces = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (p, d) = (pieces.clone(), done.clone());
        let producer = std::thread::spawn(move || {
            for _ in 0..40 {
                f.write_all(&[7u8; OUTPUT_CHUNK]).unwrap();
                p.fetch_add(1, Ordering::SeqCst);
            }
            let saved = f.finish();
            d.store(true, Ordering::SeqCst);
            saved
        });
        std::thread::sleep(Duration::from_millis(600));
        // At most the credits, the queue and the pieces being handed over are written ahead of the host.
        let ahead = pieces.load(Ordering::SeqCst);
        assert!(ahead >= u64::from(OUTPUT_CREDIT), "{ahead}");
        assert!(
            ahead <= u64::from(OUTPUT_CREDIT) + QUEUE as u64 + 2,
            "the producer ran {ahead} pieces ahead"
        );
        assert!(!done.load(Ordering::SeqCst), "the producer must wait for credit");
        go_tx.send(()).unwrap();
        let saved = producer.join().unwrap().unwrap();
        assert_eq!(
            saved,
            SavedOutput {
                name: "n.bin".into(),
                bytes: 40 * OUTPUT_CHUNK as u64
            }
        );
    }

    #[test]
    fn a_refused_stream_fails_at_once_and_an_abandoned_one_tells_the_host() {
        let (seen_tx, seen_rx) = mpsc::channel::<String>();
        let seen_tx = Mutex::new(seen_tx);
        let host: Arc<dyn Handler> = Arc::new(move |ctx: &CallCtx, body: &[u8]| {
            let HostRequest::SaveOutput { stream, name, .. } = decode_request(body)? else {
                return Err(RemoteError::new(code::UNIMPLEMENTED, "no"));
            };
            if name == "bad" {
                return encode_reply(&Err::<SavedOutput, _>("invalid output file name \"bad\"".to_string()));
            }
            let mut s = ctx
                .peer()
                .stream_request(&PluginRequest::PullOutput { stream }, OUTPUT_CREDIT)
                .map_err(|e| RemoteError::new(code::INTERNAL, e.to_string()))?;
            let why = loop {
                match s.next(None) {
                    Ok(StreamEvent::Done(_)) => break "done".to_string(),
                    Ok(_) => {}
                    Err(e) => break e.to_string(),
                }
            };
            let _ = lock(&seen_tx).send(why.clone());
            encode_reply(&Err::<SavedOutput, _>(why))
        });
        let (plugin, host_peer) = pair(host);
        let outputs = HostOutputs {
            peer: plugin.clone(),
            tool: "t".into(),
        };
        assert!(failure(outputs.create("f", "bad", 10)).contains("invalid output file name"));
        let mut f = outputs.create("f", "x", 10).unwrap();
        f.write_all(b"partial").unwrap();
        drop(f);
        let why = seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(why.contains("discarded the output"), "{why}");
        // Pulling a stream that does not exist is refused.
        let ctx_free: Result<OutputEnd, _> =
            host_peer.request(&PluginRequest::PullOutput { stream: 999_999 }, Some(Duration::from_secs(5)));
        assert!(matches!(ctx_free, Err(IpcError::Remote(e)) if e.message.contains("no output stream")));
    }

    #[test]
    fn folder_outputs_check_names_never_replace_and_clean_up() {
        let dir = tempfile::tempdir().unwrap();
        let out = FolderOutputs(dir.path().to_path_buf());
        let mut f = out.create("dotnet", "a.nettrace", 100).unwrap();
        f.write_all(b"Nettrace").unwrap();
        let saved = f.finish().unwrap();
        assert_eq!(
            saved,
            SavedOutput {
                name: "a.nettrace".into(),
                bytes: 8
            }
        );
        let path = dir.path().join("captures").join("dotnet").join("a.nettrace");
        assert_eq!(std::fs::read(&path).unwrap(), b"Nettrace");
        // Never replaced.
        assert!(failure(out.create("dotnet", "a.nettrace", 100)).contains("already exists"));
        // Bad names.
        for (folder, name) in [
            ("..", "x"),
            ("a/b", "x"),
            ("d", "../x"),
            ("d", "C:x"),
            ("d", "con.txt"),
            ("d", ".x"),
            ("d", ""),
        ] {
            assert!(out.create(folder, name, 10).is_err(), "{folder} {name}");
        }
        // Over the limit: cut off, and dropping it leaves nothing.
        let mut f = out.create("dotnet", "b.bin", 4).unwrap();
        assert!(f.write_all(b"12345").unwrap_err().to_string().contains("limit of 4 bytes"));
        drop(f);
        let names: Vec<String> = std::fs::read_dir(dir.path().join("captures").join("dotnet"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.nettrace"]);
    }
}
