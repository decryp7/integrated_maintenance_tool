//! Test harness for IMT plugins (ADR 0027). Add it as a dev-dependency and test the plugin's real binary, out of
//! process, exactly as IMT runs it: the same launcher, the same sandbox (by default the third-party one, contained
//! before the program starts), the same frames, and a fake host that lends files and views the way IMT does.
//!
//! ```no_run
//! use imt_plugin_test::{Golden, PluginUnderTest};
//!
//! #[test]
//! fn imports_the_sample() {
//!     let plugin = PluginUnderTest::launch(env!("CARGO_BIN_EXE_my-plugin")).unwrap();
//!     plugin.conformance().assert_passed();
//!     let out = plugin.import_file("tests/samples/app.log").unwrap();
//!     Golden::new("tests/golden").check("app", &out.records);
//! }
//! # fn main() {}
//! ```
//!
//! - [`PluginUnderTest`]: start the binary, call its importers, transforms, annotators and tools;
//! - [`PluginUnderTest::conformance`]: the checks IMT relies on ([`conformance`]);
//! - [`Golden`]: compare results with files kept next to the tests (`IMT_BLESS=1` rewrites them).

#![deny(missing_docs)]

pub mod conformance;
mod golden;
mod host;
mod view;

pub use conformance::{Check, Report};
pub use golden::Golden;
pub use host::TaskReport;

use anyhow::{anyhow, bail, Context, Result};
use host::FakeHost;
use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::process::SpawnSpec;
use imt_ipc::{IpcError, Json, StreamEvent};
use imt_plugin_api::annotate::{Annotation, RecordCursor};
use imt_plugin_api::manifest::{Manifest, Permission};
use imt_plugin_api::problems::Problem;
use imt_plugin_api::protocol::{
    FileInfo, HostFileRef, ImportDone, PluginRequest, TimestampSettings, ToolFail, TransformOut, WireEvent, WireViewRecord,
};
use imt_plugin_api::sandbox::{container_name, sandbox_for, sandbox_for_third_party, PluginHome};
use imt_plugin_api::{Record, PROTOCOL};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one call may take before the harness calls the plugin hung.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes of a file a sniff sees (as IMT sends).
pub const SNIFF_BYTES: usize = 64 * 1024;
/// Items an import may have in flight (as IMT grants).
const CREDIT: u32 = 4;

/// How [`PluginUnderTest::launch_with`] starts the plugin.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The permissions granted (`None`: everything the manifest asks for).
    pub granted: Option<Vec<Permission>>,
    /// Run it as IMT runs an installed third-party plugin: contained by the launcher (the default). `false` runs it
    /// as IMT runs its own plugins, which lock themselves down.
    pub third_party: bool,
    /// Largest frame on the connection.
    pub max_frame: usize,
    /// The timestamp settings importers receive (default: no extra formats, UTC).
    pub timestamps: TimestampSettings,
}

impl Default for Launch {
    fn default() -> Self {
        Launch {
            granted: None,
            third_party: true,
            max_frame: imt_ipc::frame::DEFAULT_MAX_FRAME,
            timestamps: TimestampSettings {
                formats: Vec::new(),
                zone: "UTC".into(),
                day_first: None,
            },
        }
    }
}

/// What an import returned.
#[derive(Debug, Clone, Default)]
pub struct Imported {
    /// The records, in order.
    pub records: Vec<Record>,
    /// Lines or entries skipped.
    pub skipped: u64,
    /// Problems the importer reported.
    pub problems: Vec<Problem>,
    /// The number of records in each streamed item.
    pub items: Vec<usize>,
}

/// A plugin binary running under the harness. Stopped on drop.
pub struct PluginUnderTest {
    child: Child,
    host: Arc<FakeHost>,
    manifest: Manifest,
    granted: Vec<Permission>,
    launch: Launch,
    /// The program folder and data folder; removed on drop.
    home: tempfile::TempDir,
    exe: PathBuf,
}

/// An empty manifest, for the first start (the sandbox of a plugin whose manifest is not known yet: no grants).
fn unknown_manifest() -> Manifest {
    Manifest {
        id: "unknown".into(),
        name: "unknown".into(),
        version: "0.0.0".into(),
        protocol: PROTOCOL,
        permissions: Vec::new(),
        memory_limit: None,
        importers: Vec::new(),
        config_parsers: Vec::new(),
        marker_kinds: Vec::new(),
        annotators: Vec::new(),
        transforms: Vec::new(),
        settings: None,
        tools: Vec::new(),
        profile_importers: Vec::new(),
        document_importers: Vec::new(),
    }
}

fn ipc(e: IpcError) -> anyhow::Error {
    anyhow!("{e}")
}

impl PluginUnderTest {
    /// Starts the plugin at `exe` as an installed third-party plugin, granted everything it asks for.
    pub fn launch(exe: impl AsRef<Path>) -> Result<PluginUnderTest> {
        PluginUnderTest::launch_with(exe, Launch::default())
    }

    /// Starts the plugin at `exe` as `launch` says. The program is copied into a private folder first (a
    /// contained plugin can only run from folders granted to it, as in IMT's plugin folder).
    pub fn launch_with(exe: impl AsRef<Path>, launch: Launch) -> Result<PluginUnderTest> {
        let exe = exe.as_ref();
        let home = tempfile::Builder::new().prefix("imt-plugin-test-").tempdir()?;
        let bin = home.path().join("bin");
        let data = home.path().join("data");
        std::fs::create_dir_all(&bin)?;
        std::fs::create_dir_all(&data)?;
        let name = exe.file_name().ok_or_else(|| anyhow!("{} is not a program", exe.display()))?;
        let copied = bin.join(name);
        std::fs::copy(exe, &copied).with_context(|| format!("cannot copy {}", exe.display()))?;
        let host = Arc::new(FakeHost::default());
        // First in the strictest sandbox, to learn what it is and asks for (as IMT does when installing).
        let probe = start(&copied, &unknown_manifest(), &[], &launch, home.path(), &host)?;
        let json: Json = probe.peer().request(&PluginRequest::Manifest, Some(CALL_TIMEOUT)).map_err(ipc)?;
        probe.stop();
        let manifest: Manifest = serde_json::from_value(json.0).context("the plugin's manifest is not valid")?;
        manifest.check().map_err(|e| anyhow!(e))?;
        let granted = launch
            .granted
            .clone()
            .unwrap_or_else(|| manifest.permissions.iter().map(|p| p.permission).collect());
        let child = start(&copied, &manifest, &granted, &launch, home.path(), &host)?;
        Ok(PluginUnderTest {
            child,
            host,
            manifest,
            granted,
            launch,
            home,
            exe: copied,
        })
    }

    /// The plugin's manifest, as it described itself.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The plugin's private data folder (tools receive it as their data folder).
    pub fn data_dir(&self) -> PathBuf {
        self.home.path().join("data")
    }

    /// The program the harness runs (its copy).
    pub fn program(&self) -> &Path {
        &self.exe
    }

    /// The permissions it was granted.
    pub fn granted(&self) -> &[Permission] {
        &self.granted
    }

    /// The task reports its tools sent so far.
    pub fn task_reports(&self) -> Vec<TaskReport> {
        self.host.tasks.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Whether its process is still running.
    pub fn alive(&self) -> bool {
        !self.child.exited()
            && self
                .child
                .peer()
                .request::<_, Json>(&PluginRequest::Manifest, Some(Duration::from_secs(10)))
                .is_ok()
    }

    /// Sends `req` and decodes the answer (no callbacks beyond those the fake host serves).
    pub fn request<Rep: serde::de::DeserializeOwned>(&self, req: &PluginRequest) -> Result<Rep, IpcError> {
        self.child.peer().request(req, Some(CALL_TIMEOUT))
    }

    /// Sends raw bytes as a request body (for conformance checks of malformed requests).
    pub fn request_raw(&self, body: Vec<u8>) -> Result<Vec<u8>, IpcError> {
        self.child.peer().call(body, Some(CALL_TIMEOUT))
    }

    fn file_info(&self, file_name: &str) -> FileInfo {
        FileInfo {
            name: file_name.into(),
            path: file_name.into(),
            date: None,
            timestamps: self.launch.timestamps.clone(),
        }
    }

    fn is_binary(&self, importer: &str) -> Result<bool> {
        self.manifest
            .importers
            .iter()
            .find(|i| &*i.id == importer)
            .map(|i| i.binary)
            .ok_or_else(|| anyhow!("the plugin has no importer \"{importer}\""))
    }

    /// What IMT sends a text importer: the decoded text (UTF-8, UTF-16, BOMs), or the bytes for a binary one.
    fn content(&self, importer: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        if self.is_binary(importer)? {
            return Ok(bytes.to_vec());
        }
        imt_plugin_api::text::decode(bytes)
            .map(String::into_bytes)
            .ok_or_else(|| anyhow!("IMT would not hand this file to a text importer (it is not text)"))
    }

    /// How sure `importer` is that it reads a file named `file_name` that starts with `bytes`.
    pub fn sniff(&self, importer: &str, file_name: &str, bytes: &[u8]) -> Result<u8> {
        let mut head = self.content(importer, &bytes[..bytes.len().min(SNIFF_BYTES)])?;
        head.truncate(SNIFF_BYTES);
        self.request(&PluginRequest::Sniff {
            importer: importer.into(),
            file: self.file_info(file_name),
            head,
        })
        .map_err(ipc)
    }

    /// Imports `bytes` (a file named `file_name`) with `importer`.
    pub fn import(&self, importer: &str, file_name: &str, bytes: &[u8]) -> Result<Imported> {
        self.import_within(importer, file_name, bytes, CALL_TIMEOUT)
    }

    /// [`PluginUnderTest::import`] that gives up (cancels) after `timeout`.
    pub fn import_within(&self, importer: &str, file_name: &str, bytes: &[u8], timeout: Duration) -> Result<Imported> {
        let content = self.content(importer, bytes)?;
        let len = content.len() as u64;
        let id = self.host.lend_file(content);
        let req = PluginRequest::Import {
            importer: importer.into(),
            file: self.file_info(file_name),
            content: HostFileRef { id, len },
        };
        let result = self.stream(&req, timeout);
        self.host.take_back(id);
        let (items, done) = result?;
        let done: ImportDone = imt_ipc::wire::decode(&done).map_err(|e| anyhow!("the plugin ended the import with {e}"))?;
        let mut out = Imported {
            skipped: done.skipped,
            problems: done.problems,
            ..Imported::default()
        };
        for item in items {
            let events: Vec<WireEvent> = imt_ipc::wire::decode(&item).map_err(|e| anyhow!("an item is not a list of events: {e}"))?;
            out.items.push(events.len());
            out.records.extend(events.into_iter().map(WireEvent::into_record));
        }
        Ok(out)
    }

    /// Imports the file at `path` with the importer that is surest it reads it (as IMT picks one).
    pub fn import_file(&self, path: impl AsRef<Path>) -> Result<Imported> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let importer = self.pick_importer(&name, &bytes)?;
        self.import(&importer, &name, &bytes)
    }

    /// The importer with the highest sniff score for the file (ties: the first declared); an error when none
    /// scores above 0.
    pub fn pick_importer(&self, file_name: &str, bytes: &[u8]) -> Result<String> {
        let mut best: Option<(u8, String)> = None;
        for i in &self.manifest.importers {
            let score = match self.sniff(&i.id, file_name, bytes) {
                Ok(s) => s,
                Err(_) if !i.binary => 0,
                Err(e) => return Err(e),
            };
            if score > 0 && best.as_ref().is_none_or(|(b, _)| score > *b) {
                best = Some((score, i.id.to_string()));
            }
        }
        best.map(|(_, id)| id)
            .ok_or_else(|| anyhow!("no importer of the plugin recognises {file_name}"))
    }

    /// A streamed call: its items and its final answer. A call that does not end within `timeout` is cancelled and
    /// counts as hung.
    pub(crate) fn stream(&self, req: &PluginRequest, timeout: Duration) -> Result<(Vec<Vec<u8>>, Vec<u8>)> {
        let mut stream = self.child.peer().stream_request(req, CREDIT).map_err(ipc)?;
        let deadline = Instant::now() + timeout;
        let mut items = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let _ = stream.cancel_and_wait();
                bail!("the plugin did not answer within {} s", timeout.as_secs());
            }
            match stream.next(Some(left.min(Duration::from_millis(200)))).map_err(ipc)? {
                StreamEvent::Idle => {}
                StreamEvent::Item(b) => items.push(b),
                StreamEvent::Done(b) => return Ok((items, b)),
            }
        }
    }

    /// Runs transform `id` over `records` from a file named `source_name`.
    pub fn transform(&self, id: &str, source_name: &str, records: &[Record]) -> Result<TransformOut> {
        self.request(&PluginRequest::Transform {
            transform: id.into(),
            source_name: source_name.into(),
            rows: records.iter().map(WireEvent::of).collect(),
        })
        .map_err(ipc)
    }

    /// Runs annotator `id` over a view of `records` (each from source `a.log`, instance `default` unless the
    /// record says otherwise in the `Instance` field).
    pub fn annotate(&self, id: &str, records: &[Record]) -> Result<Vec<Annotation>> {
        let rows = records
            .iter()
            .map(|r| WireViewRecord {
                event: WireEvent::of(r),
                source: "a.log".into(),
                instance: r.fields.get("Instance").and_then(|v| v.as_str()).unwrap_or("default").to_string(),
            })
            .collect();
        let ticket = self.host.lend_view(rows);
        let r = self.request(&PluginRequest::Annotate {
            annotator: id.into(),
            ticket,
            since: None::<RecordCursor>,
        });
        self.host.take_back(ticket);
        r.map_err(ipc)
    }

    /// Calls `op` of tool `tool` with `args`; the tool's data folder is [`PluginUnderTest::data_dir`].
    pub fn invoke(&self, tool: &str, op: &str, args: serde_json::Value) -> Result<serde_json::Value, ToolFail> {
        let reply: Result<Result<Json, ToolFail>, IpcError> = self.request(&PluginRequest::InvokeTool {
            tool: tool.into(),
            op: op.into(),
            args: Json(args),
            data_dir: Some(self.data_dir().to_string_lossy().into_owned()),
        });
        match reply {
            Ok(r) => r.map(Json::into_inner),
            Err(e) => Err(ToolFail {
                reason: None,
                message: e.to_string(),
            }),
        }
    }

    /// Stops task `task` of tool `tool`.
    pub fn stop_task(&self, tool: &str, task: &str) -> Result<(), ToolFail> {
        let reply: Result<Result<(), ToolFail>, IpcError> = self.request(&PluginRequest::StopTask {
            tool: tool.into(),
            task: task.into(),
            data_dir: Some(self.data_dir().to_string_lossy().into_owned()),
        });
        reply.unwrap_or_else(|e| {
            Err(ToolFail {
                reason: None,
                message: e.to_string(),
            })
        })
    }

    /// Applies settings values (validated by the plugin against its section).
    pub fn apply_settings(&self, values: serde_json::Value) -> Result<(), IpcError> {
        self.request(&PluginRequest::ApplySettings { values: Json(values) })
    }

    /// Runs the conformance checks with the plugin's own `samples` (files its importers should read; synthetic
    /// input when empty). See [`conformance`].
    pub fn conformance_with(&self, samples: &[PathBuf]) -> Report {
        conformance::run(self, samples)
    }

    /// [`PluginUnderTest::conformance_with`] without samples.
    pub fn conformance(&self) -> Report {
        conformance::run(self, &[])
    }

    /// Starts another instance of the same program with `launch` (for checks that need other limits).
    pub(crate) fn sibling(&self, launch: Launch) -> Result<PluginUnderTest> {
        PluginUnderTest::launch_with(&self.exe, launch)
    }

    /// How it was launched.
    pub fn launch_options(&self) -> &Launch {
        &self.launch
    }
}

impl Drop for PluginUnderTest {
    fn drop(&mut self) {
        self.child.stop();
    }
}

/// Starts the program at `exe` for `manifest` with `granted`, as `launch` says.
fn start(exe: &Path, manifest: &Manifest, granted: &[Permission], launch: &Launch, home: &Path, host: &Arc<FakeHost>) -> Result<Child> {
    let sandbox = if launch.third_party {
        sandbox_for_third_party(
            manifest,
            granted,
            &PluginHome {
                container: container_name(&format!("imt-plugin-test/{}", manifest.id)),
                program_dir: home.join("bin"),
                data_dir: home.join("data"),
            },
        )
    } else {
        sandbox_for(manifest, granted, false)
    };
    let mut s = SpawnSpec::new(exe, format!("plugin \"{}\"", manifest.id));
    s.current_dir = Some(home.join("bin"));
    s.endpoint_prefix = "imt-plugin-test".into();
    s.connect_window = Duration::from_secs(15);
    s.sandbox = sandbox;
    let mut c = ChildSpec::new(s);
    c.peer.protocol = PROTOCOL;
    c.peer.max_frame = launch.max_frame;
    launch_child(&c, host)
}

fn launch_child(c: &ChildSpec, host: &Arc<FakeHost>) -> Result<Child> {
    // A program that was copied into place a moment ago can still be open for writing in another thread (a forked
    // child of a parallel test holds the descriptor until its exec): Linux then refuses to run it ("Text file
    // busy", ETXTBSY). Waiting a moment and asking again is what a person would do.
    let mut attempt = 0;
    loop {
        match launch(c, host.clone()) {
            Ok(child) => return Ok(child),
            Err(e) if attempt < 40 && format!("{e}").contains("Text file busy") => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(anyhow!("cannot start the plugin: {e}")),
        }
    }
}
