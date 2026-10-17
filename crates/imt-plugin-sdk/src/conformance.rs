//! The conformance kit: contract tests every process plugin must pass before IMT installs it (ADR 0004).
//! The same tests run in CI against the built-in plugins served by `imt-plugin-host`.
//!
//! Checks: the handshake (and refusing an incompatible contract version and calls before it), the control
//! service, every declared capability (importers, tail, transforms, exporters, marker kinds, annotators,
//! settings), typed errors for unknown ids, never dying on fuzzed input (random bytes, invalid UTF-8,
//! huge lines, odd records), deadlines (an expired call fails fast and leaves the plugin responsive), and
//! that the process is the same one at the end (nothing crashed along the way).
//!
//! Run it with [`run`] on a listening plugin, [`run_binary`] on an executable, or the CLI
//! `imt-plugin-check <executable> [args...]`.

use crate::arrow;
use crate::client::{self, Client};
use crate::convert::{marker_kind_from, section_from, PermissionRequest};
use crate::manifest::RemoteManifest;
use crate::{code, pb};
use bytes::Bytes;
use imt_core::model::{Level, Record};
use imt_ipc::client::AuthChannel;
use imt_ipc::IpcError;
use serde_json::json;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One check's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Every check that ran.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub plugin: String,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.passed)
    }

    pub fn failures(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| !c.passed).collect()
    }

    fn check(&mut self, name: impl Into<String>, result: Result<String, String>) -> bool {
        let passed = result.is_ok();
        self.checks.push(Check {
            name: name.into(),
            passed,
            detail: result.unwrap_or_else(|e| e),
        });
        passed
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Conformance of {}:",
            if self.plugin.is_empty() { "the plugin" } else { &self.plugin }
        )?;
        for c in &self.checks {
            let mark = if c.passed { "ok  " } else { "FAIL" };
            if c.detail.is_empty() {
                writeln!(f, "  {mark} {}", c.name)?;
            } else {
                writeln!(f, "  {mark} {} — {}", c.name, c.detail)?;
            }
        }
        let failed = self.failures().len();
        write!(f, "{} checks, {failed} failed", self.checks.len())
    }
}

/// Options of a run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Fuzzed inputs per importer and transform.
    pub fuzz_cases: usize,
    /// Seed of the (deterministic) fuzzer.
    pub seed: u64,
    /// Permissions offered in the handshake.
    pub grant: Vec<PermissionRequest>,
    /// Deadline of a single fuzzed call.
    pub call_budget: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            fuzz_cases: 48,
            seed: 0x1d7_c0ffee,
            grant: Vec::new(),
            call_budget: Duration::from_secs(20),
        }
    }
}

/// Where a listening plugin is.
pub struct Endpoint {
    pub address: String,
    pub token: String,
}

/// A small deterministic generator (xorshift64*), so a failure can be reproduced from the seed.
struct Fuzz(u64);

impl Fuzz {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }

    /// An input that tends to break parsers.
    fn input(&mut self, case: usize) -> Vec<u8> {
        match case % 8 {
            0 => Vec::new(),
            1 => {
                let n = 1 + self.below(4096);
                self.bytes(n)
            }
            2 => {
                // Plausible log lines with random corruption.
                let mut s = String::new();
                for i in 0..self.below(200) {
                    s.push_str(&format!(
                        "2024-01-0{} 12:{:02}:{:02}.{:03} [{}] message {i} key={} \"quoted\n",
                        1 + self.below(9),
                        self.below(60),
                        self.below(60),
                        self.below(1000),
                        ["INF", "ERR", "WRN", "DBG", "???"][self.below(5)],
                        self.next()
                    ));
                }
                let mut b = s.into_bytes();
                for _ in 0..self.below(8) {
                    let at = self.below(b.len().max(1));
                    if at < b.len() {
                        b[at] = self.next() as u8;
                    }
                }
                b
            }
            3 => {
                let mut v = b"{\"@t\":\"2024-01-01T00:00:00Z\",\"@m\":\"x\",\"n\":".to_vec();
                v.extend(std::iter::repeat_n(b'[', 1 + self.below(5000)));
                v
            }
            4 => vec![b'a'; 200_000 + self.below(100_000)],
            5 => vec![0xff, 0xfe, 0x00, 0xd8, 0x00, 0xdc, 0x41, 0x00],
            6 => "a,b,c\n\"unterminated,1,2\n3,4\n,,,,,,,,,,\n"
                .repeat(1 + self.below(50))
                .into_bytes(),
            _ => {
                let n = 1 + self.below(512);
                let mut v = self.bytes(n);
                v.splice(0..0, b"\x00\x01\x02\x03ElfFile\x00".iter().copied());
                v
            }
        }
    }

    fn record(&mut self, i: usize) -> Record {
        let mut r = Record::new(i as u32);
        r.ts = if self.below(4) == 0 { None } else { Some(self.next() as i64 >> 20) };
        r.level = Level::ALL[self.below(6)];
        r.message = match self.below(4) {
            0 => String::new(),
            1 => "x".repeat(self.below(100_000)),
            2 => "naïve ☃ \u{0}\u{7f} line\nbreak".into(),
            _ => format!("message {}", self.next()),
        };
        if self.below(2) == 0 {
            r.exception = Some("System.Exception: boom\n   at A.B()".into());
        }
        if self.below(2) == 0 {
            r.fields.insert("n".into(), json!(self.next() as i64));
            r.fields.insert("s".into(), json!("text"));
            r.fields.insert("nested".into(), json!({"a": [1, 2.5, null, true]}));
        }
        r
    }
}

fn ctx(file_name: &str) -> pb::ImportContext {
    pb::ImportContext {
        file_name: file_name.into(),
        path: String::new(),
        file_date: "2024-01-01".into(),
        profile_json: String::new(),
    }
}

fn short(e: &IpcError) -> String {
    e.to_string().chars().take(300).collect()
}

/// Whether `e` is a typed error (the plugin handled the input) rather than a dead or hung process.
fn handled(e: &IpcError) -> bool {
    matches!(e, IpcError::Remote(r) if r.code != code::CRASHED)
}

struct Run<'a> {
    rt: &'a tokio::runtime::Runtime,
    client: Client,
    control: imt_ipc::pb::control_client::ControlClient<AuthChannel>,
    opts: Options,
    fuzz: Fuzz,
    report: Report,
}

impl Run<'_> {
    fn pid(&mut self) -> Result<u32, IpcError> {
        let mut c = self.control.clone();
        self.rt
            .block_on(async move { tokio::time::timeout(Duration::from_secs(5), c.status(imt_ipc::pb::StatusRequest {})).await })
            .map_err(|_| IpcError::Timeout)?
            .map(|r| r.into_inner().pid)
            .map_err(|s| client::error(&s, || "the plugin closed the connection".into()))
    }

    fn budget(&self) -> Option<Duration> {
        Some(self.opts.call_budget)
    }

    fn handshake(&mut self) -> Option<RemoteManifest> {
        // Calls before the handshake are refused with a typed error.
        let mut c = self.client.clone();
        let early = self.rt.block_on(client::marker_kinds(&mut c, Some(Duration::from_secs(5))));
        self.report.check(
            "calls before the handshake are refused",
            match early {
                Err(IpcError::Remote(r)) if r.code == code::NO_HANDSHAKE => Ok(String::new()),
                other => Err(format!("expected a NO_HANDSHAKE error, got {other:?}")),
            },
        );
        // An incompatible contract version is refused, and the plugin stays up.
        let mut c = self.client.clone();
        let wrong = self.rt.block_on(async move {
            c.handshake(pb::HandshakeRequest {
                contract_major: 99,
                ..Default::default()
            })
            .await
        });
        self.report.check(
            "an incompatible contract version is refused",
            match wrong {
                Err(s) => match imt_ipc::error::remote_error(&s) {
                    Some(r) if r.code == code::VERSION => Ok(String::new()),
                    _ => Err(format!("expected a VERSION error, got {s:?}")),
                },
                Ok(_) => Err("the plugin accepted imt.plugin.v99".into()),
            },
        );
        let mut c = self.client.clone();
        let grant = self.opts.grant.clone();
        let m = self.rt.block_on(client::handshake(&mut c, &grant, Some(Duration::from_secs(10))));
        match m {
            Ok(m) => {
                self.report.plugin = format!("{} {}", m.id, m.version);
                let caps = &m.capabilities;
                self.report.check(
                    "handshake",
                    Ok(format!(
                        "{} importer(s), {} transform(s), {} exporter(s), {} annotator(s){}{}",
                        caps.importers.len(),
                        caps.transforms.len(),
                        caps.exporters.len(),
                        caps.annotators.len(),
                        if caps.marker_kinds { ", marker kinds" } else { "" },
                        if caps.settings { ", settings" } else { "" },
                    )),
                );
                Some(m)
            }
            Err(e) => {
                self.report.check("handshake", Err(short(&e)));
                None
            }
        }
    }

    fn importers(&mut self, m: &RemoteManifest) {
        for cap in m.capabilities.importers.clone() {
            let name = format!("importer \"{}\"", cap.id);
            let mut c = self.client.clone();
            let sniff = self.rt.block_on(client::sniff(
                &mut c,
                pb::SniffRequest {
                    importer_id: cap.id.clone(),
                    ctx: Some(ctx("conformance.log")),
                    head: Bytes::from_static(b""),
                },
                Some(Duration::from_secs(5)),
            ));
            self.report.check(
                format!("{name}: sniff answers within its budget"),
                sniff.map(|s| format!("score {s}")).map_err(|e| short(&e)),
            );
            let mut failures = Vec::new();
            let mut records = 0usize;
            for case in 0..self.opts.fuzz_cases {
                let input = Bytes::from(self.fuzz.input(case));
                let file = ["fuzz.log", "fuzz.json", "fuzz.csv", "fuzz.evtx", "fuzz.txt"][case % 5];
                let mut c = self.client.clone();
                let sniffed = self.rt.block_on(client::sniff(
                    &mut c,
                    pb::SniffRequest {
                        importer_id: cap.id.clone(),
                        ctx: Some(ctx(file)),
                        head: input.slice(..input.len().min(64 * 1024)),
                    },
                    Some(Duration::from_secs(5)),
                ));
                if let Err(e) = &sniffed {
                    if !handled(e) {
                        failures.push(format!("sniff case {case}: {}", short(e)));
                        continue;
                    }
                }
                let start = pb::ImportStart {
                    importer_id: cap.id.clone(),
                    ctx: Some(ctx(file)),
                    total_bytes: input.len() as u64,
                    batch_rows: 0,
                };
                let parts = vec![(pb::ChunkPart::Content, input.clone())];
                let budget = self.budget();
                let r = self.rt.block_on(client::import(&mut c, false, start, parts, budget, |d| {
                    records += d.records.len();
                    Ok(())
                }));
                if let Err(e) = &r {
                    if !handled(e) {
                        failures.push(format!("import case {case} ({} bytes): {}", input.len(), short(e)));
                    }
                }
                if cap.tail && case % 4 == 0 {
                    let half = input.len() / 2;
                    let start = pb::ImportStart {
                        importer_id: cap.id.clone(),
                        ctx: Some(ctx(file)),
                        total_bytes: input.len() as u64,
                        batch_rows: 0,
                    };
                    let parts = vec![
                        (pb::ChunkPart::Head, input.slice(..half)),
                        (pb::ChunkPart::Tail, input.slice(half..)),
                    ];
                    let r = self.rt.block_on(client::import(&mut c, true, start, parts, budget, |_| Ok(())));
                    if let Err(e) = &r {
                        if !handled(e) {
                            failures.push(format!("import_tail case {case}: {}", short(e)));
                        }
                    }
                }
            }
            self.report.check(
                format!("{name}: never fails on {} fuzzed inputs", self.opts.fuzz_cases),
                if failures.is_empty() {
                    Ok(format!("{records} records"))
                } else {
                    Err(failures.join("; "))
                },
            );
        }
    }

    fn transforms(&mut self, m: &RemoteManifest) {
        for id in m.capabilities.transforms.clone() {
            let records: Vec<Record> = (0..self.opts.fuzz_cases.max(1)).map(|i| self.fuzz.record(i)).collect();
            let encoded = arrow::encode(&records, &[]).map(|b| (b, records.len()));
            let Ok(batch) = encoded else {
                self.report
                    .check(format!("transform \"{id}\""), Err("cannot encode test records".into()));
                continue;
            };
            let mut c = self.client.clone();
            let budget = self.budget();
            let r = self.rt.block_on(client::transform(
                &mut c,
                &id,
                "conformance.log",
                vec![batch.clone(), batch],
                budget,
            ));
            self.report.check(
                format!("transform \"{id}\": answers every batch with as many rows"),
                match r {
                    Ok(out) if out.len() == 2 && out.iter().all(|t| t.records.len() == records.len()) => Ok(format!(
                        "{} kept of {}",
                        out[0].verdicts.iter().filter(|v| **v == imt_core::plugin::Verdict::Keep).count(),
                        records.len()
                    )),
                    Ok(out) => Err(format!("{} batches back", out.len())),
                    Err(e) if handled(&e) => Ok(format!("refused with a typed error: {}", short(&e))),
                    Err(e) => Err(short(&e)),
                },
            );
        }
    }

    fn exporters(&mut self, m: &RemoteManifest) {
        for cap in m.capabilities.exporters.clone() {
            let records: Vec<Record> = (0..10).map(|i| self.fuzz.record(i)).collect();
            let rows: Vec<arrow::WithExtra<'_, 2>> = records
                .iter()
                .map(|r| arrow::WithExtra {
                    record: r,
                    extra: ["conformance.log", "group"],
                })
                .collect();
            let batch = arrow::encode(&rows, &["source_name", "group"]);
            let mut c = self.client.clone();
            let budget = self.budget();
            let r = self.rt.block_on(async {
                let batch = batch.map_err(|e| IpcError::Protocol(e.to_string()))?;
                let mut call = client::start_export(&mut c, &cap.id, budget).await?;
                call.input
                    .send(pb::ExportIn {
                        item: Some(pb::export_in::Item::Batch(pb::RecordBatch { arrow: batch, rows: 10 })),
                    })
                    .await
                    .map_err(|_| IpcError::Protocol("the export input closed".into()))?;
                drop(call.input);
                let mut bytes = 0usize;
                while let Some(m) = tokio_stream::StreamExt::next(&mut call.output).await {
                    match m.map_err(|s| client::error(&s, String::new))?.item {
                        Some(pb::export_out::Item::Data(d)) => bytes += d.len(),
                        Some(pb::export_out::Item::Done(d)) => return Ok((d.rows, bytes)),
                        None => {}
                    }
                }
                Err(IpcError::Protocol("the export ended without ExportDone".into()))
            });
            self.report.check(
                format!("exporter \"{}\": writes every row", cap.id),
                match r {
                    Ok((10, bytes)) => Ok(format!("{bytes} bytes")),
                    Ok((n, _)) => Err(format!("reported {n} rows of 10")),
                    Err(e) => Err(short(&e)),
                },
            );
        }
    }

    fn markers_annotators_settings(&mut self, m: &RemoteManifest) {
        if m.capabilities.marker_kinds {
            let mut c = self.client.clone();
            let r = self.rt.block_on(client::marker_kinds(&mut c, Some(Duration::from_secs(10))));
            self.report.check(
                "marker kinds are valid",
                r.map_err(|e| short(&e)).and_then(|kinds| {
                    let mut reg = imt_core::PluginRegistry::empty();
                    let parsed = kinds
                        .iter()
                        .map(marker_kind_from)
                        .collect::<anyhow::Result<Vec<_>>>()
                        .map_err(|e| format!("{e:#}"))?;
                    let n = parsed.len();
                    reg.add_marker_kinds(parsed).map_err(|e| format!("{e:#}"))?;
                    Ok(format!("{n} kind(s)"))
                }),
            );
        }
        for a in m.capabilities.annotators.clone() {
            let records: Vec<Record> = (0..20).map(|i| self.fuzz.record(i)).collect();
            let rows: Vec<arrow::WithExtra<'_, 2>> = records
                .iter()
                .map(|r| arrow::WithExtra {
                    record: r,
                    extra: ["conformance.log", "conformance.log"],
                })
                .collect();
            let batch = arrow::encode(&rows, &["source_name", "instance"]);
            let mut c = self.client.clone();
            let start = pb::AnnotateStart {
                annotator_id: a.id.clone(),
                instances: vec!["conformance.log".into()],
                ..Default::default()
            };
            let r = match batch {
                Ok(b) => self
                    .rt
                    .block_on(client::annotate(&mut c, start, vec![b], Some(Duration::from_secs(10)))),
                Err(e) => Err(IpcError::Protocol(e.to_string())),
            };
            self.report.check(
                format!("annotator \"{}\" answers", a.id),
                match r {
                    Ok(list) => list
                        .iter()
                        .map(crate::convert::annotation_from)
                        .collect::<anyhow::Result<Vec<_>>>()
                        .map(|l| format!("{} annotation(s)", l.len()))
                        .map_err(|e| format!("{e:#}")),
                    Err(e) if handled(&e) => Ok(format!("typed error: {}", short(&e))),
                    Err(e) => Err(short(&e)),
                },
            );
        }
        if m.capabilities.settings {
            let mut c = self.client.clone();
            let r = self.rt.block_on(client::settings_section(&mut c, Some(Duration::from_secs(10))));
            let id = m.id.clone();
            let section = r
                .map_err(|e| short(&e))
                .and_then(|s| section_from(&id, &s).map_err(|e| format!("{e:#}")));
            let ok = self.report.check(
                "the settings schema is valid",
                section.clone().map(|s| format!("{} field(s)", s.fields.len())),
            );
            if ok {
                let defaults = section.map(|s| imt_core::settings::validate(&s, &json!({})).map(|v| v.to_json()));
                if let Ok(Ok(values)) = defaults {
                    let mut c = self.client.clone();
                    let r = self
                        .rt
                        .block_on(client::apply_settings(&mut c, &values, Some(Duration::from_secs(10))));
                    self.report.check(
                        "applying the default settings succeeds",
                        r.map(|_| String::new()).map_err(|e| short(&e)),
                    );
                }
            }
        }
    }

    fn unknown_ids_and_deadlines(&mut self, m: &RemoteManifest) {
        let mut c = self.client.clone();
        let r = self.rt.block_on(client::sniff(
            &mut c,
            pb::SniffRequest {
                importer_id: "no.such.importer".into(),
                ctx: Some(ctx("x.log")),
                head: Bytes::new(),
            },
            Some(Duration::from_secs(5)),
        ));
        self.report.check(
            "unknown ids are typed errors",
            match r {
                Err(IpcError::Remote(e)) if e.code == code::INVALID => Ok(String::new()),
                other => Err(format!("expected an INVALID error, got {other:?}")),
            },
        );
        if let Some(cap) = m.capabilities.importers.first() {
            // A call whose deadline has passed fails fast, and the plugin keeps answering.
            let big = Bytes::from("2024-01-01 00:00:00 INF line\n".repeat(400_000));
            let mut c = self.client.clone();
            let started = Instant::now();
            let start = pb::ImportStart {
                importer_id: cap.id.clone(),
                ctx: Some(ctx("deadline.log")),
                total_bytes: big.len() as u64,
                batch_rows: 0,
            };
            let r = self.rt.block_on(client::import(
                &mut c,
                false,
                start,
                vec![(pb::ChunkPart::Content, big)],
                Some(Duration::from_millis(1)),
                |_| Ok(()),
            ));
            let elapsed = started.elapsed();
            let alive = self.pid();
            self.report.check(
                "an expired deadline fails fast and the plugin stays responsive",
                match (r, alive) {
                    (_, Err(e)) => Err(format!("the plugin did not answer afterwards: {}", short(&e))),
                    (Err(IpcError::Timeout), Ok(_)) if elapsed < Duration::from_secs(5) => Ok(format!("{} ms", elapsed.as_millis())),
                    (Ok(_), Ok(_)) => Ok("finished within 1 ms".into()),
                    (Err(e), Ok(_)) => Err(format!("expected a timeout, got {} after {} ms", short(&e), elapsed.as_millis())),
                },
            );
        }
    }
}

/// Runs the kit against a plugin listening at `endpoint`.
pub fn run(endpoint: &Endpoint, opts: Options) -> Report {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let mut r = Report::default();
            r.check("start", Err(format!("cannot start a runtime: {e}")));
            return r;
        }
    };
    let channel = rt
        .block_on(imt_ipc::client::connect_channel(endpoint.address.clone()))
        .map_err(|e| IpcError::Died(e.to_string()))
        .and_then(|c| imt_ipc::client::authenticated(c, &endpoint.token));
    match channel {
        Ok(ch) => run_on(&rt, ch, opts),
        Err(e) => {
            let mut r = Report::default();
            r.check("connect", Err(short(&e)));
            r
        }
    }
}

fn run_on(rt: &tokio::runtime::Runtime, channel: AuthChannel, opts: Options) -> Report {
    let seed = opts.seed;
    let mut run = Run {
        rt,
        client: client::client(channel.clone()),
        control: imt_ipc::pb::control_client::ControlClient::new(channel),
        opts,
        fuzz: Fuzz(seed | 1),
        report: Report::default(),
    };
    let pid = run.pid();
    if !run.report.check(
        "the control service answers",
        pid.clone().map(|p| format!("pid {p}")).map_err(|e| short(&e)),
    ) {
        return run.report;
    }
    let Some(m) = run.handshake() else {
        return run.report;
    };
    run.importers(&m);
    run.transforms(&m);
    run.exporters(&m);
    run.markers_annotators_settings(&m);
    run.unknown_ids_and_deadlines(&m);
    let after = run.pid();
    run.report.check(
        "the process never crashed",
        match (pid, after) {
            (Ok(a), Ok(b)) if a == b => Ok(String::new()),
            (_, Ok(b)) => Err(format!("now pid {b}")),
            (_, Err(e)) => Err(short(&e)),
        },
    );
    run.report
}

/// Starts `exe` the way IMT does (private address, token on stdin) and runs the kit against it.
pub fn run_binary(exe: impl Into<PathBuf>, args: &[String], opts: Options) -> Report {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let mut r = Report::default();
            r.check("start", Err(format!("cannot start a runtime: {e}")));
            return r;
        }
    };
    let mut spec = imt_ipc::process::SpawnSpec::new(exe, "the plugin");
    spec.args = args.to_vec();
    spec.endpoint_prefix = "imt-check".into();
    spec.clear_env = true;
    match rt.block_on(imt_ipc::process::launch(&spec)) {
        Ok(l) => {
            let report = run_on(&rt, l.channel.clone(), opts);
            l.process.close(Duration::from_secs(2));
            report
        }
        Err(e) => {
            let mut r = Report::default();
            r.check("start the plugin", Err(short(&e)));
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fuzzer_is_deterministic_and_varied() {
        let mut a = Fuzz(7);
        let mut b = Fuzz(7);
        let xs: Vec<Vec<u8>> = (0..16).map(|i| a.input(i)).collect();
        let ys: Vec<Vec<u8>> = (0..16).map(|i| b.input(i)).collect();
        assert_eq!(xs, ys);
        assert!(xs[0].is_empty() && xs[4].len() >= 200_000);
        let r = a.record(3);
        assert_eq!(r.line, 3);
        assert_eq!(a.below(0), 0);
    }

    #[test]
    fn reports_print_and_count() {
        let mut r = Report::default();
        assert!(!r.passed());
        r.check("a", Ok(String::new()));
        r.check("b", Ok("detail".into()));
        assert!(r.passed());
        r.check("c", Err("broke".into()));
        assert!(!r.passed());
        assert_eq!(r.failures().len(), 1);
        let text = r.to_string();
        assert!(text.contains("ok   a") && text.contains("FAIL c — broke") && text.ends_with("3 checks, 1 failed"));
        assert!(handled(&IpcError::Remote(imt_ipc::RemoteError::new(code::INVALID, "x"))));
        assert!(!handled(&IpcError::Remote(imt_ipc::RemoteError::new(code::CRASHED, "x"))));
        assert!(!handled(&IpcError::Timeout));
    }

    #[test]
    fn unreachable_plugins_are_reported() {
        let r = run(
            &Endpoint {
                address: "/nonexistent/imt.sock".into(),
                token: "t".into(),
            },
            Options::default(),
        );
        assert!(!r.passed());
        assert_eq!(r.checks[0].name, "connect");
        let r = run_binary("/nonexistent/plugin", &[], Options::default());
        assert!(!r.passed() && r.checks[0].name == "start the plugin");
    }
}
