//! Logs of the child processes Debrief supervises (the AI worker and its llama-server, `debrief-plugin-host` per
//! isolated plugin, out-of-process plugins), for "Report a problem".
//!
//! Each child has one [`ChildLog`]: its standard error (scrubbed of obvious secrets, lines capped at
//! [`MAX_LINE`]) plus Debrief's own lines about it — started (pid, run), exited (exit code or signal, crash or
//! normal), stopped by Debrief, quarantined. Lines are CLEF (`{"@t":..,"@l":..,"@m":..}`), so Debrief opens these
//! logs like its own, and the messages are worded so the default profile marks them: `Started …` is a start
//! marker, `Stopped …` a stop marker, a crash is `Fatal`, a quarantine an `Error`.
//!
//! Files: `<dir>/<name>.log` for this Debrief session, capped at [`MAX_FILE_BYTES`] (the oldest half is dropped
//! when it is reached), and the two previous sessions' as `<name>.1.log` and `<name>.2.log` ([`KEEP_RUNS`]).
//! The folder is set once by the app ([`set_dir`]); before that (and in tests) logs are kept in memory only,
//! bounded to [`RING_BYTES`].
//!
//! Never logged: the child's environment (the llama-server API key is passed there), its arguments, file
//! contents or prompts. Children must not print those either (llama-server runs with errors-only logging).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Largest log file of one child per session; the oldest half is dropped when it is reached.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Sessions kept per child: this one and the two before.
pub const KEEP_RUNS: usize = 3;
/// Longest line kept (bytes); longer ones are cut at a character boundary.
pub const MAX_LINE: usize = 4096;
/// In-memory tail per child (when there is no folder, and for live summaries).
pub const RING_BYTES: usize = 64 * 1024;
/// Most children tracked at once (one per plugin process at most, plus the AI engine).
pub const MAX_CHILDREN: usize = 256;
/// Prefix of the lines a child forwards from its own child (`[llama-server] text`).
pub const FORWARD_OPEN: char = '[';
/// Marker inside a forwarded line that carries a lifecycle event (`[llama-server] @debrief start 42`).
pub const EVENT_TAG: &str = "@debrief ";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a child is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChildKind {
    AiWorker,
    LlamaServer,
    PluginHost,
    Plugin,
    /// The store worker (DuckDB and the full-text indexes of every window, ADR 0014).
    StoreWorker,
}

impl ChildKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChildKind::AiWorker => "ai-worker",
            ChildKind::LlamaServer => "llama-server",
            ChildKind::PluginHost => "plugin-host",
            ChildKind::Plugin => "plugin",
            ChildKind::StoreWorker => "store-worker",
        }
    }

    /// Words for summaries ("AI engine: 2 runs").
    pub fn title(self) -> &'static str {
        match self {
            ChildKind::AiWorker => "AI engine",
            ChildKind::LlamaServer => "llama-server",
            ChildKind::PluginHost => "Isolated plugin",
            ChildKind::Plugin => "Plugin process",
            ChildKind::StoreWorker => "Store worker",
        }
    }
}

/// Which log a child writes to, and the logs of its own children (lines it forwards as `[<name>] text`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildLogSpec {
    pub name: String,
    pub kind: ChildKind,
    pub version: Option<String>,
    pub forwarded: Vec<ChildLogSpec>,
}

impl ChildLogSpec {
    /// A spec whose name is made safe for a file name (see [`log_name`]).
    pub fn new(name: impl AsRef<str>, kind: ChildKind) -> ChildLogSpec {
        ChildLogSpec {
            name: log_name(name.as_ref()),
            kind,
            version: None,
            forwarded: Vec::new(),
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn forwarding(mut self, child: ChildLogSpec) -> Self {
        self.forwarded.push(child);
        self
    }
}

/// A file-safe log name: `[A-Za-z0-9._-]`, other characters become `-`, at most 64 characters, never empty,
/// never starting with a dot.
pub fn log_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
        if out.len() >= 64 {
            break;
        }
    }
    let out = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if out.is_empty() {
        "child".into()
    } else {
        out
    }
}

/// How a child ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Code(i64),
    Signal(i32),
    /// Debrief stopped it (closed its lifeline, killed it).
    StoppedByDebrief,
    Unknown,
}

impl Exit {
    pub fn of(status: &ExitStatus) -> Exit {
        if let Some(code) = status.code() {
            return Exit::Code(i64::from(code));
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                return Exit::Signal(sig);
            }
        }
        Exit::Unknown
    }

    /// A non-zero exit or a signal the child did not get from Debrief.
    pub fn is_crash(&self) -> bool {
        matches!(self, Exit::Code(c) if *c != 0) || matches!(self, Exit::Signal(_))
    }

    /// "exit code 3", "exit code 0xC0000005", "signal 11", "stopped by Debrief".
    pub fn describe(&self) -> String {
        match self {
            // Windows NTSTATUS crash codes read better in hex.
            Exit::Code(c) if *c < 0 || *c > 0xFFFF => format!("exit code 0x{:08X}", *c as u32),
            Exit::Code(c) => format!("exit code {c}"),
            Exit::Signal(s) => format!("signal {s}"),
            Exit::StoppedByDebrief => "stopped by Debrief".into(),
            Exit::Unknown => "unknown".into(),
        }
    }

    /// Parses [`Exit::describe`]'s words after `@debrief exit <pid> ` (`code 3`, `signal 11`, `stopped`).
    fn parse_wire(words: &str) -> Exit {
        let mut it = words.split_whitespace();
        match (it.next(), it.next()) {
            (Some("code"), Some(n)) => n.parse().map(Exit::Code).unwrap_or(Exit::Unknown),
            (Some("signal"), Some(n)) => n.parse().map(Exit::Signal).unwrap_or(Exit::Unknown),
            (Some("stopped"), _) => Exit::StoppedByDebrief,
            _ => Exit::Unknown,
        }
    }

    /// The words a child forwarding its own child's exit writes after `@debrief exit <pid> `.
    pub fn wire(&self) -> String {
        match self {
            Exit::Code(c) => format!("code {c}"),
            Exit::Signal(s) => format!("signal {s}"),
            Exit::StoppedByDebrief => "stopped".into(),
            Exit::Unknown => "unknown".into(),
        }
    }
}

/// State of a child as summarised for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChildRunState {
    Running,
    /// Exited on its own with code 0.
    Exited,
    /// Debrief stopped it.
    Stopped,
    Crashed,
    Quarantined,
    /// Started, but the log ends without an exit line (Debrief itself ended first, e.g. it crashed).
    Unknown,
}

/// One child in a report's `processes.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSummary {
    pub name: String,
    pub kind: ChildKind,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub state: ChildRunState,
    /// Starts in the session the log covers; the first is not a restart.
    pub starts: u32,
    pub restarts: u32,
    pub last_exit: Option<String>,
    pub last_exit_at: Option<String>,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
}

impl ProcessSummary {
    fn new(name: &str, kind: ChildKind) -> ProcessSummary {
        ProcessSummary {
            name: name.to_string(),
            kind,
            version: None,
            pid: None,
            state: ChildRunState::Unknown,
            starts: 0,
            restarts: 0,
            last_exit: None,
            last_exit_at: None,
            quarantined: false,
            quarantine_reason: None,
        }
    }

    /// "AI engine: 2 runs, 1 restart, last exit code 3, quarantined".
    pub fn describe(&self) -> String {
        let mut s = format!(
            "{}: {} run{}",
            self.kind.title(),
            self.starts,
            if self.starts == 1 { "" } else { "s" }
        );
        if self.restarts > 0 {
            s.push_str(&format!(", {} restart{}", self.restarts, if self.restarts == 1 { "" } else { "s" }));
        }
        if let Some(e) = &self.last_exit {
            s.push_str(&format!(", last {e}"));
        }
        if self.quarantined {
            s.push_str(", quarantined");
        } else if self.state == ChildRunState::Running {
            s.push_str(", running");
        }
        s
    }

    fn apply(&mut self, line: &LogLine) {
        if let Some(v) = &line.version {
            self.version = Some(v.clone());
        }
        match line.event.as_deref() {
            Some("start") => {
                self.starts += 1;
                self.restarts = self.starts.saturating_sub(1);
                self.pid = line.pid;
                self.state = ChildRunState::Running;
                self.quarantined = false;
                self.quarantine_reason = None;
            }
            Some(e @ ("exit" | "crash" | "stop")) => {
                self.state = match e {
                    "crash" => ChildRunState::Crashed,
                    "stop" => ChildRunState::Stopped,
                    _ => ChildRunState::Exited,
                };
                self.last_exit = line.exit.clone();
                self.last_exit_at = line.time.clone();
                if line.pid.is_some() {
                    self.pid = line.pid;
                }
            }
            Some("quarantine") => {
                self.state = ChildRunState::Quarantined;
                self.quarantined = true;
                self.quarantine_reason = line.reason.clone();
            }
            _ => {}
        }
    }
}

/// The properties Debrief writes on its own lines (others are ignored).
#[derive(Debug, Default, Deserialize)]
struct LogLine {
    #[serde(rename = "@t")]
    time: Option<String>,
    #[serde(rename = "Event")]
    event: Option<String>,
    #[serde(rename = "ChildKind")]
    kind: Option<ChildKind>,
    #[serde(rename = "Pid")]
    pid: Option<u32>,
    #[serde(rename = "Exit")]
    exit: Option<String>,
    #[serde(rename = "Reason")]
    reason: Option<String>,
    #[serde(rename = "Version")]
    version: Option<String>,
}

/// Summarises a child's log text (one session's file). A log that ends on a start is [`ChildRunState::Unknown`]:
/// whether it still runs is only known live (see [`ChildLog::status`]). `None` when no line is Debrief's.
pub fn summarize(name: &str, text: &str) -> Option<ProcessSummary> {
    let mut summary: Option<ProcessSummary> = None;
    for raw in text.lines() {
        let Ok(line) = serde_json::from_str::<LogLine>(raw) else { continue };
        if line.event.as_deref().is_none_or(|e| e == "output" || e == "note") {
            continue;
        }
        let kind = line.kind.unwrap_or(ChildKind::Plugin);
        summary.get_or_insert_with(|| ProcessSummary::new(name, kind)).apply(&line);
    }
    if let Some(s) = summary.as_mut() {
        if s.state == ChildRunState::Running {
            s.state = ChildRunState::Unknown;
        }
    }
    summary
}

/// UTC RFC 3339 with milliseconds (`2026-10-14T09:12:03.120Z`).
pub fn rfc3339(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (y, m, day) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60,
        d.subsec_millis()
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

// ---- scrubbing ----

/// Words that name a secret when they are a flag or a key (`--api-key x`, `token=x`, `"password": "x"`).
const SECRET_WORDS: &[&str] = &[
    "apikey",
    "api-key",
    "api_key",
    "token",
    "password",
    "passwd",
    "pwd",
    "secret",
    "authorization",
    "bearer",
    "credential",
    "credentials",
    "cookie",
];

fn names_secret(key: &str) -> bool {
    let k = key
        .trim_matches(|c: char| c == '-' || c == '"' || c == '\'' || c == '{' || c == ',' || c == ' ')
        .to_ascii_lowercase();
    !k.is_empty()
        && SECRET_WORDS
            .iter()
            .any(|w| k == *w || k.ends_with(&format!("-{w}")) || k.ends_with(&format!("_{w}")) || k.ends_with(w) && w.len() > 5)
}

/// A run of characters that looks like a key or token: 32+ hex digits, or 40+ letters/digits/`-_` mixing
/// upper case, lower case and digits.
fn looks_like_token(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    if w.len() >= 32 && w.chars().all(|c| c.is_ascii_hexdigit()) && w.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    w.len() >= 40
        && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_uppercase())
        && w.chars().any(|c| c.is_ascii_lowercase())
}

const REDACTED: &str = "***";

/// One line of a child's output made safe to keep: control characters removed, obvious secrets replaced by
/// `***` (values after secret-named flags and keys, `Bearer` tokens, long key-like runs), cut to [`MAX_LINE`].
pub fn scrub(line: &str) -> String {
    let clean: String = line
        .trim_end_matches(['\r', '\n'])
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let mut out = String::with_capacity(clean.len());
    let mut redact_next = false;
    for (i, word) in clean.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if word.is_empty() {
            continue;
        }
        if redact_next {
            // `Authorization: Bearer <token>`: the scheme stays, the token goes.
            if matches!(word.to_ascii_lowercase().as_str(), "bearer" | "basic") {
                out.push_str(word);
            } else {
                out.push_str(REDACTED);
                redact_next = false;
            }
            continue;
        }
        if let Some(pos) = word.find(['=', ':']) {
            let (key, rest) = word.split_at(pos);
            if names_secret(key) {
                out.push_str(key);
                out.push_str(&rest[..1]);
                if rest.len() > 1 {
                    out.push_str(REDACTED);
                } else {
                    // `token: value` / `"apiKey": "value"`.
                    redact_next = true;
                }
                continue;
            }
        }
        if names_secret(word) {
            out.push_str(word);
            redact_next = true;
            continue;
        }
        if looks_like_token(word) {
            out.push_str(REDACTED);
            continue;
        }
        out.push_str(word);
    }
    cap_line(out)
}

fn cap_line(mut s: String) -> String {
    if s.len() > MAX_LINE {
        let mut end = MAX_LINE;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

// ---- lines ----

/// What happened, as Debrief writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildEvent {
    Start {
        pid: u32,
    },
    /// A line of the child's standard error.
    Output(String),
    Exit {
        pid: Option<u32>,
        exit: Exit,
    },
    Quarantined {
        reason: String,
    },
    /// Debrief's own remark (the log was cut, ...).
    Note(String),
}

fn level_of_output(text: &str) -> &'static str {
    let l = text.to_ascii_lowercase();
    if l.contains("panicked") || l.contains("fatal") || l.contains("error") {
        "Error"
    } else if l.contains("warn") {
        "Warning"
    } else {
        "Information"
    }
}

/// The CLEF line for one event (no trailing newline).
fn format_line(at: SystemTime, name: &str, kind: ChildKind, version: Option<&str>, run: u32, event: &ChildEvent) -> String {
    let mut v = serde_json::Map::new();
    v.insert("@t".into(), rfc3339(at).into());
    let (level, message, tag) = match event {
        ChildEvent::Start { pid } => ("Information", format!("Started {name} (pid {pid}, run {run})"), "start"),
        ChildEvent::Output(text) => (level_of_output(text), scrub(text), "output"),
        ChildEvent::Exit { pid, exit } => {
            let pid_text = pid.map(|p| format!(" (pid {p})")).unwrap_or_default();
            match exit {
                Exit::StoppedByDebrief => ("Information", format!("Stopped {name}{pid_text} by Debrief"), "stop"),
                e if e.is_crash() => ("Fatal", format!("Crashed: {name}{pid_text} ended with {}", e.describe()), "crash"),
                e => ("Information", format!("Stopped {name}{pid_text}: {}", e.describe()), "exit"),
            }
        }
        // The reason stays out of the message: its words ("was stopped") must not read as a stop marker.
        ChildEvent::Quarantined { .. } => (
            "Error",
            format!("Quarantined {name}: no more restarts until it is reset"),
            "quarantine",
        ),
        ChildEvent::Note(text) => ("Information", scrub(text), "note"),
    };
    v.insert("@l".into(), level.into());
    v.insert("@m".into(), message.into());
    v.insert("Child".into(), name.into());
    v.insert("ChildKind".into(), kind.as_str().into());
    v.insert("Event".into(), tag.into());
    match event {
        ChildEvent::Start { pid } => {
            v.insert("Pid".into(), (*pid).into());
            v.insert("Run".into(), run.into());
            v.insert("Restarts".into(), run.saturating_sub(1).into());
            if let Some(ver) = version {
                v.insert("Version".into(), ver.into());
            }
        }
        ChildEvent::Exit { pid, exit } => {
            if let Some(p) = pid {
                v.insert("Pid".into(), (*p).into());
            }
            v.insert("Exit".into(), exit.describe().into());
            v.insert("Crash".into(), exit.is_crash().into());
            v.insert("Run".into(), run.into());
        }
        ChildEvent::Quarantined { reason } => {
            v.insert("Reason".into(), cap_line(scrub(reason)).into());
        }
        _ => {}
    }
    serde_json::Value::Object(v).to_string()
}

// ---- sinks ----

/// Where one child's lines go: its file (when there is a folder) and a bounded in-memory tail.
struct Sink {
    file: Option<(PathBuf, std::fs::File)>,
    written: u64,
    ring: VecDeque<String>,
    ring_bytes: usize,
}

/// The file of a child's log for the current session.
pub fn log_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.log"))
}

fn rotated_path(dir: &Path, name: &str, n: usize) -> PathBuf {
    dir.join(format!("{name}.{n}.log"))
}

/// Shifts `<name>.log` → `<name>.1.log` → … keeping [`KEEP_RUNS`] sessions in all. Never fails.
pub fn rotate(dir: &Path, name: &str) {
    let current = log_path(dir, name);
    if !current.exists() {
        return;
    }
    let oldest = KEEP_RUNS - 1;
    let _ = std::fs::remove_file(rotated_path(dir, name, oldest));
    for n in (1..oldest).rev() {
        let _ = std::fs::rename(rotated_path(dir, name, n), rotated_path(dir, name, n + 1));
    }
    let _ = std::fs::rename(&current, rotated_path(dir, name, 1));
}

impl Sink {
    fn open(dir: Option<&Path>, name: &str) -> Sink {
        let file = dir.and_then(|d| {
            std::fs::create_dir_all(d).ok()?;
            rotate(d, name);
            let path = log_path(d, name);
            let f = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
            Some((path, f))
        });
        Sink {
            file,
            written: 0,
            ring: VecDeque::new(),
            ring_bytes: 0,
        }
    }

    fn push(&mut self, line: &str, note_cut: impl Fn() -> String) {
        self.ring_bytes += line.len() + 1;
        self.ring.push_back(line.to_string());
        while self.ring_bytes > RING_BYTES {
            match self.ring.pop_front() {
                Some(old) => self.ring_bytes -= old.len() + 1,
                None => break,
            }
        }
        let needed = line.len() as u64 + 1;
        if self.file.is_some() && self.written + needed > MAX_FILE_BYTES {
            self.compact(&note_cut());
        }
        if let Some((_, f)) = self.file.as_mut() {
            if f.write_all(format!("{line}\n").as_bytes()).is_ok() {
                self.written += needed;
            }
        }
    }

    /// Keeps the newest half of the file (whole lines) after a note that the rest was cut.
    fn compact(&mut self, note: &str) {
        let Some((path, _)) = &self.file else { return };
        let path = path.clone();
        let keep = MAX_FILE_BYTES / 2;
        let tail = (|| -> std::io::Result<Vec<u8>> {
            let mut f = std::fs::File::open(&path)?;
            let len = f.metadata()?.len();
            f.seek(SeekFrom::Start(len.saturating_sub(keep)))?;
            let mut buf = Vec::new();
            f.take(keep).read_to_end(&mut buf)?;
            Ok(buf)
        })()
        .unwrap_or_default();
        let start = tail.iter().position(|b| *b == b'\n').map(|p| p + 1).unwrap_or(tail.len());
        let mut data = format!("{note}\n").into_bytes();
        data.extend_from_slice(&tail[start..]);
        let reopened = std::fs::write(&path, &data)
            .and_then(|_| std::fs::OpenOptions::new().append(true).open(&path))
            .ok();
        self.written = data.len() as u64;
        self.file = reopened.map(|f| (path, f));
    }
}

// ---- one child ----

struct Live {
    summary: ProcessSummary,
    run: u32,
}

/// One child's log; see the module docs. Cheap to share (`Arc`); every method is infallible.
pub struct ChildLog {
    name: String,
    kind: ChildKind,
    version: Mutex<Option<String>>,
    live: Mutex<Live>,
    sink: Mutex<Sink>,
    forwarded: Vec<Arc<ChildLog>>,
}

impl ChildLog {
    /// A log writing into `dir` (rotating the previous sessions' files), or kept in memory without one.
    /// Not registered: use [`get`] for the logs of running children.
    pub fn open(dir: Option<&Path>, spec: &ChildLogSpec) -> ChildLog {
        ChildLog {
            name: spec.name.clone(),
            kind: spec.kind,
            version: Mutex::new(spec.version.clone()),
            live: Mutex::new(Live {
                summary: ProcessSummary {
                    version: spec.version.clone(),
                    ..ProcessSummary::new(&spec.name, spec.kind)
                },
                run: 0,
            }),
            sink: Mutex::new(Sink::open(dir, &spec.name)),
            forwarded: spec.forwarded.iter().map(get).collect(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> ChildKind {
        self.kind
    }

    /// The file this session writes, if any.
    pub fn path(&self) -> Option<PathBuf> {
        lock(&self.sink).file.as_ref().map(|(p, _)| p.clone())
    }

    /// Records the version once it is known (a plugin's, after its handshake).
    pub fn set_version(&self, version: &str) {
        *lock(&self.version) = Some(version.to_string());
        lock(&self.live).summary.version = Some(version.to_string());
    }

    /// Writes one event at `at` (fixed times make reproducible logs).
    pub fn record_at(&self, at: SystemTime, event: &ChildEvent) {
        let line = {
            let mut live = lock(&self.live);
            if let ChildEvent::Start { .. } = event {
                live.run += 1;
            }
            let version = lock(&self.version).clone();
            let line = format_line(at, &self.name, self.kind, version.as_deref(), live.run, event);
            if let Ok(parsed) = serde_json::from_str::<LogLine>(&line) {
                live.summary.apply(&parsed);
            }
            line
        };
        let name = self.name.clone();
        let kind = self.kind;
        lock(&self.sink).push(&line, || {
            format_line(
                at,
                &name,
                kind,
                None,
                0,
                &ChildEvent::Note(format!(
                    "Earlier output was cut to keep this log under {} KB",
                    MAX_FILE_BYTES / 1024
                )),
            )
        });
    }

    pub fn record(&self, event: &ChildEvent) {
        self.record_at(SystemTime::now(), event);
    }

    pub fn started(&self, pid: u32) {
        self.record(&ChildEvent::Start { pid });
    }

    pub fn exited(&self, pid: Option<u32>, exit: Exit) {
        self.record(&ChildEvent::Exit { pid, exit });
    }

    pub fn quarantined(&self, reason: &str) {
        self.record(&ChildEvent::Quarantined {
            reason: reason.to_string(),
        });
    }

    /// One line of the child's standard error. A line `[<name>] text` from a child that forwards its own
    /// child's output goes to that log; `[<name>] @debrief start <pid>` / `@debrief exit <pid> code 3` are its events.
    pub fn output(&self, raw: &str) {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.starts_with(FORWARD_OPEN) {
            if let Some((tag, rest)) = line[1..].split_once("] ") {
                if let Some(target) = self.forwarded.iter().find(|f| f.name == tag) {
                    target.forwarded_line(rest);
                    return;
                }
            }
        }
        self.record(&ChildEvent::Output(line.to_string()));
    }

    fn forwarded_line(&self, rest: &str) {
        let Some(ev) = rest.strip_prefix(EVENT_TAG) else {
            return self.record(&ChildEvent::Output(rest.to_string()));
        };
        let mut it = ev.splitn(3, ' ');
        match (it.next(), it.next().and_then(|p| p.parse::<u32>().ok()), it.next()) {
            (Some("start"), Some(pid), _) => self.started(pid),
            (Some("exit"), pid, words) => self.exited(pid, Exit::parse_wire(words.unwrap_or(""))),
            _ => self.record(&ChildEvent::Output(rest.to_string())),
        }
    }

    /// The child's state now.
    pub fn status(&self) -> ProcessSummary {
        lock(&self.live).summary.clone()
    }

    /// The newest lines kept in memory (bounded to [`RING_BYTES`]).
    pub fn tail_text(&self) -> String {
        let sink = lock(&self.sink);
        let mut s = String::with_capacity(sink.ring_bytes);
        for l in &sink.ring {
            s.push_str(l);
            s.push('\n');
        }
        s
    }
}

// ---- the registry ----

#[derive(Default)]
struct Registry {
    dir: Option<PathBuf>,
    children: BTreeMap<String, Arc<ChildLog>>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

/// Sets the folder of the children's logs (`<Debrief log folder>/children`). Children started before keep
/// their logs in memory.
pub fn set_dir(dir: PathBuf) {
    lock(registry()).dir = Some(dir);
}

/// The folder set with [`set_dir`].
pub fn dir() -> Option<PathBuf> {
    lock(registry()).dir.clone()
}

/// The log of a child (created on first use; one per name for the whole session, so restarts append).
pub fn get(spec: &ChildLogSpec) -> Arc<ChildLog> {
    let (existing, dir) = {
        let r = lock(registry());
        (r.children.get(&spec.name).cloned(), r.dir.clone())
    };
    if let Some(l) = existing {
        if let Some(v) = &spec.version {
            l.set_version(v);
        }
        return l;
    }
    // Opened outside the lock: a forwarding child registers its own children first.
    let log = Arc::new(ChildLog::open(dir.as_deref(), spec));
    let mut r = lock(registry());
    if r.children.len() >= MAX_CHILDREN && !r.children.contains_key(&spec.name) {
        // Bounded: beyond the limit a log is kept but not listed.
        return log;
    }
    r.children.entry(spec.name.clone()).or_insert(log).clone()
}

/// The registered log of that name, if a child of that name has started in this session.
pub fn find(name: &str) -> Option<Arc<ChildLog>> {
    lock(registry()).children.get(name).cloned()
}

/// Every registered child's state now.
pub fn statuses() -> Vec<ProcessSummary> {
    let logs: Vec<Arc<ChildLog>> = lock(registry()).children.values().cloned().collect();
    logs.iter().map(|l| l.status()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        // 2026-10-14T09:00:00Z
        UNIX_EPOCH + Duration::from_secs(1_791_968_400 + secs)
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-child-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(rfc3339(at(0)), "2026-10-14T09:00:00.000Z");
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_millis(951_782_400_123)),
            "2000-02-29T00:00:00.123Z"
        );
        assert_eq!(rfc3339(UNIX_EPOCH + Duration::from_secs(4_107_542_399)), "2100-02-28T23:59:59.000Z");
    }

    #[test]
    fn names_are_file_safe() {
        assert_eq!(log_name("plugin \"net.decryptology.web\""), "plugin-net.decryptology.web");
        assert_eq!(log_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(log_name(""), "child");
        assert_eq!(log_name("...."), "child");
        assert!(log_name(&"x".repeat(500)).len() <= 64);
    }

    #[test]
    fn secrets_are_scrubbed() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef";
        for (input, want) in [
            (format!("using --api-key {key} now"), "using --api-key *** now".to_string()),
            ("token=abc123 ok".into(), "token=*** ok".into()),
            ("LLAMA_API_KEY=supersecret".into(), "LLAMA_API_KEY=***".into()),
            ("Authorization: Bearer xyz".into(), "Authorization: Bearer ***".into()),
            ("header Bearer xyz.abc".into(), "header Bearer ***".into()),
            ("{\"password\": \"hunter2\"}".into(), "{\"password\": ***".into()),
            (format!("key {key}"), "key ***".into()),
            (
                "model loaded from C:/models/qwen2.5-7b.gguf in 3.2 s".into(),
                "model loaded from C:/models/qwen2.5-7b.gguf in 3.2 s".into(),
            ),
            ("tab\there\u{1b}[0m".into(), "tab here[0m".into()),
        ] {
            assert_eq!(scrub(&input), want, "{input}");
        }
        assert!(scrub(&"é".repeat(MAX_LINE)).len() <= MAX_LINE + '…'.len_utf8());
        // Mixed-case long runs (base64-ish keys) too, but not ordinary words.
        assert_eq!(scrub("sk-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcd"), "***");
        assert_eq!(scrub("Started worker"), "Started worker");
    }

    #[test]
    fn exits_are_classified_and_described() {
        assert!(!Exit::Code(0).is_crash());
        assert!(Exit::Code(3).is_crash() && Exit::Signal(11).is_crash());
        assert!(!Exit::StoppedByDebrief.is_crash() && !Exit::Unknown.is_crash());
        assert_eq!(Exit::Code(-1073741819).describe(), "exit code 0xC0000005");
        assert_eq!(Exit::Code(3).describe(), "exit code 3");
        assert_eq!(Exit::Signal(11).describe(), "signal 11");
        for e in [Exit::Code(3), Exit::Signal(9), Exit::StoppedByDebrief, Exit::Unknown] {
            assert_eq!(Exit::parse_wire(&e.wire()), e);
        }
        assert_eq!(Exit::parse_wire("code x"), Exit::Unknown);
    }

    #[test]
    fn lines_are_clef_and_marked_by_the_default_profile_words() {
        let spec = ChildLogSpec::new("ai-worker", ChildKind::AiWorker).with_version("0.1.0");
        let log = ChildLog::open(None, &spec);
        log.record_at(at(0), &ChildEvent::Start { pid: 42 });
        log.record_at(at(1), &ChildEvent::Output("loading model".into()));
        log.record_at(
            at(2),
            &ChildEvent::Exit {
                pid: Some(42),
                exit: Exit::Code(3),
            },
        );
        log.record_at(at(3), &ChildEvent::Start { pid: 43 });
        log.record_at(
            at(4),
            &ChildEvent::Exit {
                pid: Some(43),
                exit: Exit::StoppedByDebrief,
            },
        );
        log.record_at(at(5), &ChildEvent::Start { pid: 44 });
        log.record_at(
            at(6),
            &ChildEvent::Exit {
                pid: Some(44),
                exit: Exit::Code(0),
            },
        );
        let text = log.tail_text();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["@m"], "Started ai-worker (pid 42, run 1)");
        assert_eq!(lines[0]["@t"], "2026-10-14T09:00:00.000Z");
        assert_eq!(lines[0]["Version"], "0.1.0");
        assert_eq!(lines[2]["@l"], "Fatal");
        assert_eq!(lines[2]["@m"], "Crashed: ai-worker (pid 42) ended with exit code 3");
        assert_eq!(lines[2]["Crash"], true);
        assert_eq!(lines[3]["Restarts"], 1);
        assert_eq!(lines[4]["@m"], "Stopped ai-worker (pid 43) by Debrief");
        assert_eq!(lines[6]["@m"], "Stopped ai-worker (pid 44): exit code 0");
        assert_eq!(lines[6]["Event"], "exit");
        let live = log.status();
        assert_eq!((live.starts, live.restarts, live.state), (3, 2, ChildRunState::Exited));
        // The same summary from the file text, except that a final start is unknown, not running.
        assert_eq!(summarize("ai-worker", &text).unwrap(), live);
        assert!(summarize("x", "not json\n{\"Event\":\"output\"}\n").is_none());
    }

    #[test]
    fn quarantine_keeps_its_reason_out_of_the_message() {
        let log = ChildLog::open(None, &ChildLogSpec::new("plugin-x", ChildKind::Plugin));
        log.record_at(at(0), &ChildEvent::Start { pid: 7 });
        log.record_at(
            at(1),
            &ChildEvent::Quarantined {
                reason: "failed 3 times and was stopped; token=abc".into(),
            },
        );
        let line: serde_json::Value = serde_json::from_str(log.tail_text().lines().last().unwrap()).unwrap();
        assert_eq!(line["@l"], "Error");
        assert!(!line["@m"].as_str().unwrap().contains("stopped"));
        assert_eq!(line["Reason"], "failed 3 times and was stopped; token=***");
        let s = log.status();
        assert!(s.quarantined && s.state == ChildRunState::Quarantined);
        assert!(s.describe().ends_with("quarantined"), "{}", s.describe());
        let s = summarize("plugin-x", &log.tail_text()).unwrap();
        assert_eq!(s.quarantine_reason.as_deref(), Some("failed 3 times and was stopped; token=***"));
    }

    #[test]
    fn forwarded_lines_go_to_the_grandchilds_log() {
        let llama = ChildLogSpec::new("llama-server-fwd-test", ChildKind::LlamaServer);
        let worker = ChildLog::open(
            None,
            &ChildLogSpec::new("ai-worker-fwd-test", ChildKind::AiWorker).forwarding(llama),
        );
        worker.output("[llama-server-fwd-test] @debrief start 99\n");
        worker.output("[llama-server-fwd-test] main: loading model");
        worker.output("[llama-server-fwd-test] @debrief exit 99 signal 11");
        worker.output("[llama-server-fwd-test] @debrief bogus");
        worker.output("[other] stays with the worker");
        let llama = find("llama-server-fwd-test").unwrap();
        let s = llama.status();
        assert_eq!(
            (s.pid, s.state, s.last_exit.as_deref()),
            (Some(99), ChildRunState::Crashed, Some("signal 11"))
        );
        assert!(llama.tail_text().contains("main: loading model"));
        assert!(llama.tail_text().contains("@debrief bogus"));
        assert!(worker.tail_text().contains("[other] stays"));
        assert!(statuses().iter().any(|s| s.name == "llama-server-fwd-test"));
    }

    #[test]
    fn files_are_capped_and_rotated_over_three_sessions() {
        let d = temp("rotate");
        let spec = ChildLogSpec::new("w", ChildKind::Plugin);
        for session in 0..5 {
            let log = ChildLog::open(Some(&d), &spec);
            log.record_at(at(session), &ChildEvent::Note(format!("session {session}")));
        }
        let mut names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["w.1.log", "w.2.log", "w.log"]);
        assert!(std::fs::read_to_string(d.join("w.log")).unwrap().contains("session 4"));
        assert!(std::fs::read_to_string(d.join("w.2.log")).unwrap().contains("session 2"));

        // The current file never grows past the cap and keeps the newest lines.
        let log = ChildLog::open(Some(&d), &spec);
        let filler = "x".repeat(1000);
        for i in 0..3000 {
            log.record_at(at(i), &ChildEvent::Output(format!("{i} {filler}")));
        }
        let len = std::fs::metadata(d.join("w.log")).unwrap().len();
        assert!(len <= MAX_FILE_BYTES, "{len}");
        let text = std::fs::read_to_string(d.join("w.log")).unwrap();
        assert!(text.lines().next().unwrap().contains("Earlier output was cut"));
        assert!(text.contains("\"2999 x"));
        assert!(text.lines().all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()));
        assert!(log.tail_text().len() <= RING_BYTES);
        assert_eq!(log.path(), Some(d.join("w.log")));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_registry_appends_restarts_to_one_log() {
        let spec = ChildLogSpec::new("registry-test", ChildKind::PluginHost);
        let a = get(&spec);
        a.started(1);
        let b = get(&spec.clone().with_version("2.0"));
        b.started(2);
        assert!(Arc::ptr_eq(&a, &b));
        let s = find("registry-test").unwrap().status();
        assert_eq!((s.starts, s.restarts, s.version.as_deref()), (2, 1, Some("2.0")));
        assert_eq!(a.name(), "registry-test");
        assert_eq!(a.kind(), ChildKind::PluginHost);
        assert!(find("never-started").is_none());
    }
}
