//! The conformance checks: what IMT relies on from every plugin, run against the real binary under the sandbox it
//! gets in IMT.
//!
//! | Check | What passes |
//! |---|---|
//! | `manifest` | the manifest is valid for this IMT (protocol, ids, contributions), every permission has a reason, and a third-party plugin asks only for `network` and `runPrograms` |
//! | `permissions` | the plugin runs with exactly what it declares: every check below runs in that sandbox, so an undeclared right (a file outside its folders, the network, another program) fails there |
//! | `typed-errors` | an unknown importer, tool or transform and a request that does not decode are answered with a typed error, and the plugin keeps serving |
//! | `bounded-output` | every importer streams items of at most `MAX_EVENTS_PER_ITEM` events, and its records have bounded fields |
//! | `cancellation` | an import that is cancelled ends within the cancel grace period (`imt_ipc::peer::CANCEL_GRACE`) and the plugin keeps serving |
//! | `oversized-frames` | on a connection with a small frame limit (64 KiB), an answer that does not fit is a typed error, not a hang or a crash |
//!
//! Panics: a panic in a plugin's code is answered as a typed `PANIC` error and the process keeps serving. That is
//! the SDK's behaviour for every plugin; the harness's own tests check it with a fixture that panics on request.

use crate::{Launch, PluginUnderTest};
use imt_ipc::peer::{code, CANCEL_GRACE};
use imt_ipc::{IpcError, StreamEvent};
use imt_plugin_api::protocol::{FileInfo, HostFileRef, PluginRequest, MAX_EVENTS_PER_ITEM};
use imt_plugin_package::manifest::{valid_id, PACKAGE_PERMISSIONS};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The check's name (`manifest`, `typed-errors`, ...).
    pub name: &'static str,
    /// Whether it passed.
    pub passed: bool,
    /// What was found (why it failed, or what was checked).
    pub detail: String,
}

/// The outcome of all checks.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Every check, in order.
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// The failed checks.
    pub fn failures(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| !c.passed).collect()
    }

    /// Panics with every failure (for tests).
    #[track_caller]
    pub fn assert_passed(&self) {
        assert!(self.passed(), "conformance checks failed:\n{self}");
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in &self.checks {
            writeln!(f, "{} {}: {}", if c.passed { "ok  " } else { "FAIL" }, c.name, c.detail)?;
        }
        Ok(())
    }
}

fn check(name: &'static str, r: Result<String, String>) -> Check {
    match r {
        Ok(detail) => Check {
            name,
            passed: true,
            detail,
        },
        Err(detail) => Check {
            name,
            passed: false,
            detail,
        },
    }
}

/// Synthetic text for importers when the plugin gives no samples: plain lines with a timestamp each.
fn synthetic(lines: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..lines {
        s.push_str(&format!(
            "2026-10-21T10:{:02}:{:02}Z INFO line {i} key=value\n",
            (i / 60) % 60,
            i % 60
        ));
    }
    s.into_bytes()
}

/// Runs every check against `p` (see the module docs); `samples` are files its importers should read.
pub fn run(p: &PluginUnderTest, samples: &[PathBuf]) -> Report {
    let mut r = Report::default();
    r.checks.push(check("manifest", manifest(p)));
    r.checks.push(check(
        "permissions",
        Ok(format!(
            "ran with {:?} ({})",
            p.granted(),
            if p.launch_options().third_party {
                "contained as a third-party plugin"
            } else {
                "as IMT's own plugin"
            }
        )),
    ));
    r.checks.push(check("typed-errors", typed_errors(p)));
    let inputs: Vec<(String, Vec<u8>)> = if samples.is_empty() {
        vec![("synthetic.log".into(), synthetic(2000))]
    } else {
        samples
            .iter()
            .filter_map(|s| {
                std::fs::read(s)
                    .ok()
                    .map(|b| (s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), b))
            })
            .collect()
    };
    r.checks.push(check("bounded-output", bounded(p, &inputs)));
    r.checks.push(check("cancellation", cancellation(p, &inputs)));
    r.checks.push(check("oversized-frames", oversized(p, &inputs)));
    r
}

fn manifest(p: &PluginUnderTest) -> Result<String, String> {
    let m = p.manifest();
    m.check()?;
    if !valid_id(&m.id) {
        return Err(format!("id \"{}\" must be reverse-DNS style (com.example.name)", m.id));
    }
    if semver_ok(&m.version).is_none() {
        return Err(format!("version \"{}\" is not semver", m.version));
    }
    if let Some(r) = m.permissions.iter().find(|r| r.reason.trim().is_empty()) {
        return Err(format!("permission {:?} needs a reason the user can read", r.permission));
    }
    if p.launch_options().third_party {
        if let Some(r) = m.permissions.iter().find(|r| !PACKAGE_PERMISSIONS.contains(&r.permission)) {
            return Err(format!(
                "permission {:?} is only available to IMT's own plugins; read files through the host instead",
                r.permission
            ));
        }
    }
    Ok(format!("{} {} ({} contributions)", m.id, m.version, m.contributions().len()))
}

fn semver_ok(v: &str) -> Option<()> {
    let core = v.split(['-', '+']).next()?;
    let parts: Vec<&str> = core.split('.').collect();
    (parts.len() == 3 && parts.iter().all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))).then_some(())
}

fn expect_typed(what: &str, r: Result<Vec<u8>, IpcError>) -> Result<(), String> {
    match r {
        Err(IpcError::Remote(e)) if e.code == code::INVALID || e.code == code::UNIMPLEMENTED => Ok(()),
        Err(IpcError::Remote(e)) => Err(format!(
            "{what}: answered with error code {} ({}), expected INVALID",
            e.code, e.message
        )),
        Err(e) => Err(format!("{what}: the plugin failed instead of answering with an error: {e}")),
        Ok(_) => Err(format!("{what}: the plugin accepted it")),
    }
}

fn typed_errors(p: &PluginUnderTest) -> Result<String, String> {
    let enc = |req: &PluginRequest| imt_ipc::wire::encode(req).map_err(|e| e.to_string());
    let file = FileInfo {
        name: "x.log".into(),
        path: "x.log".into(),
        date: None,
        timestamps: p.launch_options().timestamps.clone(),
    };
    expect_typed(
        "an unknown importer",
        p.request_raw(enc(&PluginRequest::Sniff {
            importer: "imt-plugin-test.no-such-importer".into(),
            file,
            head: b"x".to_vec(),
        })?),
    )?;
    expect_typed(
        "an unknown transform",
        p.request_raw(enc(&PluginRequest::Transform {
            transform: "imt-plugin-test.no-such-transform".into(),
            source_name: "x".into(),
            rows: Vec::new(),
        })?),
    )?;
    expect_typed(
        "an unknown tool",
        p.request_raw(enc(&PluginRequest::InvokeTool {
            tool: "imt-plugin-test.no-such-tool".into(),
            op: "go".into(),
            args: imt_ipc::Json(serde_json::json!({})),
            data_dir: None,
        })?),
    )?;
    expect_typed("a request that does not decode", p.request_raw(vec![0xff; 16]))?;
    if !p.alive() {
        return Err("the plugin stopped after a bad request".into());
    }
    Ok("unknown contributions and malformed requests get typed errors".into())
}

fn bounded(p: &PluginUnderTest, inputs: &[(String, Vec<u8>)]) -> Result<String, String> {
    let mut checked = 0;
    for i in &p.manifest().importers {
        for (name, bytes) in inputs {
            if !i.binary && imt_plugin_api::text::decode(bytes).is_none() {
                continue;
            }
            let out = p
                .import_within(&i.id, name, bytes, IMPORT_TIMEOUT)
                .map_err(|e| format!("importer \"{}\" on {name}: {e:#}", i.id))?;
            if let Some(n) = out.items.iter().find(|n| **n > MAX_EVENTS_PER_ITEM) {
                return Err(format!(
                    "importer \"{}\" sent an item of {n} events (at most {MAX_EVENTS_PER_ITEM})",
                    i.id
                ));
            }
            if let Some(r) = out.records.iter().find(|r| r.message.len() > imt_plugin_api::text::MAX_LINE_LEN) {
                return Err(format!(
                    "importer \"{}\" produced a message of {} bytes on line {}",
                    i.id,
                    r.message.len(),
                    r.line
                ));
            }
            checked += 1;
        }
    }
    Ok(format!("{checked} imports streamed bounded items"))
}

fn cancellation(p: &PluginUnderTest, inputs: &[(String, Vec<u8>)]) -> Result<String, String> {
    let Some(importer) = p.manifest().importers.first() else {
        return Ok("no importers".into());
    };
    // A large input: the samples repeated to about 16 MiB.
    let (name, sample) = inputs.first().cloned().unwrap_or_else(|| ("synthetic.log".into(), synthetic(2000)));
    let mut big = Vec::new();
    while big.len() < 16 << 20 && !sample.is_empty() {
        big.extend_from_slice(&sample);
        if !sample.ends_with(b"\n") && !importer.binary {
            big.push(b'\n');
        }
    }
    let content = if importer.binary {
        big
    } else {
        imt_plugin_api::text::decode(&big).map(String::into_bytes).unwrap_or(big)
    };
    let id = p.host.lend_file(content.clone());
    let req = PluginRequest::Import {
        importer: importer.id.clone(),
        file: FileInfo {
            name,
            path: "big.log".into(),
            date: None,
            timestamps: p.launch_options().timestamps.clone(),
        },
        content: HostFileRef {
            id,
            len: content.len() as u64,
        },
    };
    let result = (|| -> Result<String, String> {
        let mut stream = p.child.peer().stream_request(&req, 1).map_err(|e| e.to_string())?;
        // Give up on it once it has read the whole file (the SDK stops reads at a cancel itself): what is checked
        // is that the importer's own work ends.
        let read_before = p.host.bytes_read.load(std::sync::atomic::Ordering::Relaxed);
        let waiting = Instant::now();
        while p.host.bytes_read.load(std::sync::atomic::Ordering::Relaxed) - read_before < content.len() as u64
            && waiting.elapsed() < Duration::from_secs(10)
        {
            if let Ok(StreamEvent::Done(_)) = stream.next(Some(Duration::from_millis(20))) {
                return Ok("the import ended before it could be cancelled".into());
            }
        }
        let started = Instant::now();
        stream.cancel();
        loop {
            match stream.next(Some(Duration::from_millis(100))) {
                Ok(StreamEvent::Done(_)) => break,
                Ok(_) => {}
                Err(IpcError::Remote(e)) if e.code == code::CANCELLED => break,
                Err(IpcError::Remote(_)) => break,
                Err(e) => return Err(format!("the import broke instead of ending: {e}")),
            }
            if started.elapsed() > CANCEL_GRACE + Duration::from_secs(1) {
                return Err(format!(
                    "importer \"{}\" kept running {} s after it was cancelled (IMT allows {} s)",
                    importer.id,
                    started.elapsed().as_secs(),
                    CANCEL_GRACE.as_secs()
                ));
            }
        }
        Ok(format!("ended {} ms after the cancel", started.elapsed().as_millis()))
    })();
    p.host.take_back(id);
    let detail = result?;
    if !p.alive() {
        return Err("the plugin stopped after a cancelled import".into());
    }
    Ok(detail)
}

/// How long one import of a sample may take in the checks.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(20);

/// The frame limit of the `oversized-frames` check.
const SMALL_FRAME: usize = 64 * 1024;

fn oversized(p: &PluginUnderTest, inputs: &[(String, Vec<u8>)]) -> Result<String, String> {
    let Some(importer) = p.manifest().importers.first() else {
        return Ok("no importers".into());
    };
    let small = p
        .sibling(Launch {
            max_frame: SMALL_FRAME,
            granted: Some(p.granted().to_vec()),
            ..p.launch_options().clone()
        })
        .map_err(|e| format!("cannot start a second instance: {e:#}"))?;
    let (name, sample) = inputs.first().cloned().unwrap_or_else(|| ("synthetic.log".into(), synthetic(2000)));
    let mut big = sample.clone();
    while big.len() < 2 << 20 && !sample.is_empty() {
        big.extend_from_slice(&sample);
    }
    // Its reads and answer cannot fit in 64 KiB frames: a typed error (or an import that produced nothing) is fine, a crash or
    // a hang is not. The request itself is small: the content is read back through the host in chunks.
    let started = Instant::now();
    let r = small.import_within(&importer.id, &name, &big, IMPORT_TIMEOUT);
    if started.elapsed() > IMPORT_TIMEOUT + CANCEL_GRACE {
        return Err("the import hung on a small frame limit".into());
    }
    if !small.alive() {
        return Err(format!("the plugin died when its answer did not fit a frame ({:?})", r.err()));
    }
    Ok(match r {
        Ok(out) => format!("{} records fit in 64 KiB frames", out.records.len()),
        Err(e) => format!("refused with a typed error: {e:#}"),
    })
}
