//! Exception grouping.
//!
//! Parses stack traces from .NET, Java, Python and JavaScript into [`ExceptionInfo`] and computes a
//! fingerprint from the exception type and the top frames after normalisation, so the same bug raised
//! from different lines, instances or compiler-generated closures lands in one [`ExceptionGroup`].
//!
//! Frames are always ordered top of stack first. For chained exceptions `inner` is the cause.

use crate::row::RowRef;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Frames kept per exception, and nesting levels followed; both bound memory on hostile input.
const MAX_FRAMES: usize = 256;
const MAX_DEPTH: usize = 16;
const MAX_LINES: usize = 10_000;
const MAX_SAMPLES: usize = 10;
const MAX_FALLBACK_MESSAGE: usize = 500;
const DEFAULT_FRAMES: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Qualified function name without arguments.
    pub function: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionInfo {
    pub type_name: String,
    pub message: String,
    pub frames: Vec<Frame>,
    pub inner: Option<Box<ExceptionInfo>>,
}

impl ExceptionInfo {
    /// The deepest cause.
    pub fn root(&self) -> &ExceptionInfo {
        let mut cur = self;
        while let Some(i) = &cur.inner {
            cur = i;
        }
        cur
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionGroup {
    pub fingerprint: String,
    pub type_name: String,
    /// Message of the first record in the group.
    pub message: String,
    /// Top frame of the root cause after dropping framework noise.
    pub top_frame: Option<String>,
    pub count: usize,
    /// Occurrences per instance key (for example a source file or host), most frequent first.
    pub instances: Vec<(String, usize)>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub sample_ids: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Line classification
// ---------------------------------------------------------------------------

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static regex"))
}

fn dotnet_loc_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^at (.+?\)) in (.+?)(?::line (\d+))?$")
}
fn java_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"^at ([^\s()]+)\((?:Native Method|Unknown Source|([^\s():]+\.[A-Za-z]+)(?::(\d+))?)\)$",
    )
}
fn js_paren_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^at (.+?) \((.*)\)$")
}
fn js_bare_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^at (.+?):(\d+):(\d+)$")
}
fn py_frame_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r#"^File "(.*)", line (\d+)(?:, in (.+))?$"#)
}
fn arity_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"`\d+")
}
fn compiler_name_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^<(.+?)>\w__[\w|]*$")
}
fn java_noise_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"\$\$Lambda[\w$]*(?:/\w+)?|\$\$\w+?\$\$[0-9a-fA-F]+|\$\d+|lambda\$([\w$]+?)\$\d+",
    )
}

fn parse_u32(s: &str) -> Option<u32> {
    s.parse().ok()
}

/// Parses `line` (already trimmed) as a stack frame.
fn parse_frame(line: &str) -> Option<Frame> {
    if let Some(c) = py_frame_re().captures(line) {
        return Some(Frame {
            function: c.get(3).map_or("<module>", |m| m.as_str()).to_string(),
            file: Some(c[1].to_string()),
            line: parse_u32(&c[2]),
        });
    }
    if !line.starts_with("at ") {
        return None;
    }
    if let Some(c) = dotnet_loc_re().captures(line) {
        return Some(Frame {
            function: strip_args(&c[1]).to_string(),
            file: Some(c[2].to_string()),
            line: c.get(3).and_then(|m| parse_u32(m.as_str())),
        });
    }
    if let Some(c) = java_re().captures(line) {
        let f = &c[1];
        let function = if f.contains("$$Lambda") {
            f
        } else {
            f.rsplit('/').next().unwrap_or(f)
        };
        return Some(Frame {
            function: function.to_string(),
            file: c.get(2).map(|m| m.as_str().to_string()),
            line: c.get(3).and_then(|m| parse_u32(m.as_str())),
        });
    }
    if let Some(c) = js_paren_re().captures(line) {
        let (file, line_no) = split_location(&c[2]);
        return Some(Frame {
            function: c[1].to_string(),
            file,
            line: line_no,
        });
    }
    if let Some(c) = js_bare_re().captures(line) {
        return Some(Frame {
            function: "<anonymous>".to_string(),
            file: Some(c[1].to_string()),
            line: parse_u32(&c[2]),
        });
    }
    // .NET frame without source information: `at Ns.Class.Method(args)`.
    let rest = &line[3..];
    if rest.contains('(') && !rest.starts_with('(') {
        return Some(Frame {
            function: strip_args(rest).to_string(),
            file: None,
            line: None,
        });
    }
    None
}

/// `file:line:col` or `file:line` into file and line; anything else (`native`, `<anonymous>`, `index 0`) is kept as the file.
fn split_location(loc: &str) -> (Option<String>, Option<u32>) {
    if let Some((rest, last)) = loc.rsplit_once(':') {
        if let Some(n) = parse_u32(last) {
            return match rest.rsplit_once(':') {
                Some((file, line)) if parse_u32(line).is_some() => (Some(file.to_string()), parse_u32(line)),
                _ => (Some(rest.to_string()), Some(n)),
            };
        }
    }
    if loc.is_empty() {
        (None, None)
    } else {
        (Some(loc.to_string()), None)
    }
}

fn strip_args(s: &str) -> &str {
    s.split('(').next().unwrap_or(s).trim()
}

fn is_type_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_' || c == '$')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '+' | '`' | '<' | '>' | '[' | ']'))
}

/// Splits `Type: message`; a line that is not of that form is `Unknown` with the line as message.
fn parse_header(line: &str) -> (String, String) {
    let line = line.trim();
    let line = line.strip_prefix("--->").unwrap_or(line).trim();
    let line = line.strip_prefix("Caused by:").unwrap_or(line).trim();
    if let Some(p) = line.find(':') {
        let (ty, rest) = (line[..p].trim(), line[p + 1..].trim());
        if is_type_name(ty) && (rest.is_empty() || line[p + 1..].starts_with(' ')) {
            return (ty.to_string(), rest.to_string());
        }
    }
    if is_type_name(line) {
        return (line.to_string(), String::new());
    }
    ("Unknown".to_string(), line.to_string())
}

fn looks_like_exception_type(t: &str) -> bool {
    t.contains('.') || ["Exception", "Error", "Throwable", "Fault"].iter().any(|s| t.ends_with(s))
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

const END_MARKER: &str = "--- End of inner exception stack trace ---";

/// Parses a stack trace in any supported language. Returns `None` for text that does not look like an exception.
pub fn parse_exception(text: &str) -> Option<ExceptionInfo> {
    let raw: Vec<&str> = text.lines().take(MAX_LINES).collect();
    let lines: Vec<&str> = raw.iter().map(|l| l.trim()).collect();
    let info = if lines
        .iter()
        .any(|l| l.starts_with("Traceback (most recent call last)") || py_frame_re().is_match(l))
    {
        parse_python(&raw)?
    } else {
        let start = lines.iter().position(|l| !l.is_empty())?;
        let mut pos = start;
        parse_block(&lines, &mut pos, 0)
    };
    let has_frames = {
        let mut cur = Some(&info);
        let mut any = false;
        while let Some(i) = cur {
            any |= !i.frames.is_empty();
            cur = i.inner.as_deref();
        }
        any
    };
    (has_frames || looks_like_exception_type(&info.type_name)).then_some(info)
}

fn is_continuation_break(l: &str) -> bool {
    l.starts_with("--->") || l == END_MARKER || l.starts_with("Caused by:") || l.starts_with("Suppressed:") || parse_frame(l).is_some()
}

/// Parses one exception (header, frames, nested causes) from `lines[*pos..]`, advancing `pos`.
fn parse_block(lines: &[&str], pos: &mut usize, depth: usize) -> ExceptionInfo {
    let first = lines[*pos];
    let (type_name, mut message) = if parse_frame(first).is_some() {
        ("Unknown".to_string(), String::new())
    } else {
        *pos += 1;
        parse_header(first)
    };
    // Multi-line messages run until the first structural line.
    while *pos < lines.len() && !lines[*pos].is_empty() && !is_continuation_break(lines[*pos]) && !lines[*pos].starts_with("...") {
        message.push('\n');
        message.push_str(lines[*pos]);
        *pos += 1;
    }
    let mut info = ExceptionInfo {
        type_name,
        message,
        frames: Vec::new(),
        inner: None,
    };
    let mut skipping = false;
    while *pos < lines.len() {
        let l = lines[*pos];
        if l == END_MARKER {
            *pos += 1;
            if depth > 0 {
                return info;
            }
        } else if l.starts_with("--->") || l.starts_with("Caused by:") {
            if depth >= MAX_DEPTH {
                *pos += 1;
                continue;
            }
            skipping = false;
            let is_dotnet = l.starts_with("--->");
            let inner = parse_block(lines, pos, depth + 1);
            // A `Caused by` chain consumes the rest of the trace, so nothing follows it.
            info.inner = Some(Box::new(inner));
            if !is_dotnet {
                return info;
            }
        } else if l.starts_with("Suppressed:") {
            skipping = true;
            *pos += 1;
        } else {
            if !skipping && info.frames.len() < MAX_FRAMES {
                if let Some(f) = parse_frame(l) {
                    info.frames.push(f);
                }
            }
            *pos += 1;
        }
    }
    info
}

fn parse_python(raw: &[&str]) -> Option<ExceptionInfo> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let base = raw.iter().filter(|l| !l.trim().is_empty()).map(|l| indent(l)).min().unwrap_or(0);
    struct Seg {
        frames: Vec<Frame>,
        header: Option<(String, String)>,
        started: bool,
    }
    let new_seg = |started| Seg {
        frames: Vec::new(),
        header: None,
        started,
    };
    let mut segs: Vec<Seg> = Vec::new();
    let mut cur = new_seg(false);
    for line in raw {
        let l = line.trim();
        if l.starts_with("Traceback (most recent call last)") {
            if cur.started && (cur.header.is_some() || !cur.frames.is_empty()) {
                segs.push(cur);
            }
            cur = new_seg(true);
        } else if let Some(f) = parse_frame(l).filter(|_| py_frame_re().is_match(l)) {
            if cur.header.is_some() {
                segs.push(cur);
                cur = new_seg(true);
            }
            cur.started = true;
            if cur.frames.len() < MAX_FRAMES {
                cur.frames.push(f);
            }
        } else if cur.started
            && cur.header.is_none()
            && !cur.frames.is_empty()
            && indent(line) == base
            && !l.is_empty()
            && parse_header(l).0 != "Unknown"
        {
            cur.header = Some(parse_header(l));
        }
    }
    if cur.started {
        segs.push(cur);
    }
    let mut prev: Option<Box<ExceptionInfo>> = None;
    for seg in segs.into_iter().take(MAX_DEPTH) {
        let Some((type_name, message)) = seg.header else { continue };
        let mut frames = seg.frames;
        frames.reverse();
        prev = Some(Box::new(ExceptionInfo {
            type_name,
            message,
            frames,
            inner: prev,
        }));
    }
    prev.map(|b| *b)
}

// ---------------------------------------------------------------------------
// Fingerprinting
// ---------------------------------------------------------------------------

/// Normalises a function name so equivalent frames compare equal.
fn normalize_function(f: &str) -> String {
    let f = f.trim();
    let f = f.strip_prefix("async ").unwrap_or(f);
    let f = f.strip_prefix("new ").unwrap_or(f);
    let f = arity_re().replace_all(f, "");
    let f = strip_brackets(&f);
    let f = java_noise_re().replace_all(&f, |c: &regex::Captures| c.get(1).map_or(String::new(), |m| m.as_str().to_string()));
    let raw: Vec<&str> = f.split('.').collect();
    let mut out: Vec<String> = Vec::new();
    let mut after_state_machine = false;
    for (i, seg) in raw.iter().enumerate() {
        let seg = seg.trim();
        let mut state_machine = false;
        let name: String = if let Some(c) = compiler_name_re().captures(seg) {
            state_machine = seg.contains(">d__");
            c[1].to_string()
        } else if seg.starts_with("<>c")
            || seg.starts_with("b__")
            || seg.starts_with("lambda_method")
            || seg.is_empty()
            || (seg == "MoveNext" && after_state_machine && i + 1 == raw.len())
        {
            continue;
        } else {
            seg.to_string()
        };
        after_state_machine = state_machine;
        if out.last() != Some(&name) {
            out.push(name);
        }
    }
    out.join(".")
}

/// Removes balanced `[...]` groups (generic argument lists) from a name.
fn strip_brackets(s: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_noise_frame(f: &str) -> bool {
    f.starts_with("System.Runtime.CompilerServices.")
        || f.starts_with("System.Runtime.ExceptionServices.")
        || f.starts_with("System.Threading.ExecutionContext.")
        || f.starts_with("System.Threading.Tasks.Task.")
}

/// The first `n` normalised, non-noise frames (falling back to noise frames if nothing else is left), with the originals.
fn significant_frames(info: &ExceptionInfo, n: usize) -> Vec<(String, &Frame)> {
    let all: Vec<(String, &Frame)> = info
        .frames
        .iter()
        .map(|f| (normalize_function(&f.function), f))
        .filter(|(name, _)| !name.is_empty())
        .collect();
    let quiet: Vec<(String, &Frame)> = all.iter().filter(|(name, _)| !is_noise_frame(name)).cloned().collect();
    let chosen = if quiet.is_empty() { all } else { quiet };
    chosen.into_iter().take(n).collect()
}

fn fnv1a(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn normalize_type(t: &str) -> String {
    strip_brackets(&arity_re().replace_all(t, "")).to_string()
}

/// Stable hex fingerprint (16 digits) of the exception type and its top `frames` normalised frames.
/// For chained exceptions it covers the outer type and the root cause's type and frames.
pub fn fingerprint(info: &ExceptionInfo, frames: usize) -> String {
    let root = info.root();
    let outer = normalize_type(&info.type_name);
    let root_type = normalize_type(&root.type_name);
    let sig = significant_frames(root, frames);
    let mut parts: Vec<&str> = vec![&outer, &root_type];
    parts.extend(sig.iter().map(|(n, _)| n.as_str()));
    format!("{:016x}", fnv1a(&parts))
}

/// Fallback fingerprint for unparseable text: first line with digit runs collapsed.
fn text_fingerprint(first_line: &str) -> String {
    let mut norm = String::with_capacity(first_line.len());
    let mut in_digits = false;
    for c in first_line.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                norm.push('#');
            }
            in_digits = true;
        } else {
            norm.push(c);
            in_digits = false;
        }
    }
    format!("{:016x}", fnv1a(&["Unknown", &norm]))
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

struct Acc {
    group: ExceptionGroup,
    instances: HashMap<String, usize>,
}

/// Groups the exceptions of `records` by fingerprint. Each item pairs a record with an instance key
/// (such as its source file). Records without an exception are skipped. Result is ordered by count, most frequent first.
pub fn group<R: RowRef, K: AsRef<str>>(records: impl IntoIterator<Item = (R, K)>) -> Vec<ExceptionGroup> {
    let mut grouper = Grouper::default();
    for (r, instance) in records {
        grouper.add(r.exception().map(Parsed::of), &r, instance.as_ref());
    }
    grouper.finish()
}

/// [`group`] with the parsing and fingerprinting (the costly, pure part) done on `stages`' pool and the counting in
/// record order on one thread: the same groups as [`group`], found faster. The flag is false when cancelled.
pub fn group_pipelined<R: RowRef + Send + Sync, K: AsRef<str> + Send + Sync>(
    records: impl Iterator<Item = (R, K)>,
    stages: &crate::stages::Stages<'_>,
) -> (Vec<ExceptionGroup>, bool) {
    let mut grouper = Grouper::default();
    let done = stages.run(
        records,
        |(r, _)| r.exception().map(Parsed::of),
        |(r, instance), parsed, _| grouper.add(parsed, &r, instance.as_ref()),
    );
    (grouper.finish(), done)
}

/// What parsing one exception text yields: the fingerprint and the group's descriptive fields. Pure in the text, so
/// it can be computed on any thread before the ordered counting.
pub struct Parsed {
    fp: String,
    type_name: String,
    message: String,
    top_frame: Option<String>,
}

impl Parsed {
    /// Parses and fingerprints `text`.
    pub fn of(text: &str) -> Parsed {
        let (fp, type_name, message, top_frame) = match parse_exception(text) {
            Some(info) => {
                let top = significant_frames(info.root(), 1).first().map(|(_, f)| f.function.clone());
                (fingerprint(&info, DEFAULT_FRAMES), info.type_name, info.message, top)
            }
            None => {
                let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
                let message: String = first.chars().take(MAX_FALLBACK_MESSAGE).collect();
                (text_fingerprint(first), "Unknown".to_string(), message, None)
            }
        };
        Parsed {
            fp,
            type_name,
            message,
            top_frame,
        }
    }
}

/// Counts exceptions into groups, in the order the records arrive.
#[derive(Default)]
pub struct Grouper {
    index: HashMap<String, usize>,
    accs: Vec<Acc>,
}

impl Grouper {
    /// Counts `r` under `instance`; a record without an exception (`parsed` none) is skipped.
    pub fn add<R: RowRef + ?Sized>(&mut self, parsed: Option<Parsed>, r: &R, instance: &str) {
        let Some(Parsed {
            fp,
            type_name,
            message,
            top_frame,
        }) = parsed
        else {
            return;
        };
        let accs = &mut self.accs;
        let i = *self.index.entry(fp.clone()).or_insert_with(|| {
            accs.push(Acc {
                group: ExceptionGroup {
                    fingerprint: fp,
                    type_name,
                    message,
                    top_frame,
                    count: 0,
                    instances: Vec::new(),
                    first_ts: None,
                    last_ts: None,
                    sample_ids: Vec::new(),
                },
                instances: HashMap::new(),
            });
            accs.len() - 1
        });
        let a = &mut self.accs[i];
        a.group.count += 1;
        *a.instances.entry(instance.to_string()).or_default() += 1;
        if let Some(ts) = r.ts() {
            a.group.first_ts = Some(a.group.first_ts.map_or(ts, |t| t.min(ts)));
            a.group.last_ts = Some(a.group.last_ts.map_or(ts, |t| t.max(ts)));
        }
        if a.group.sample_ids.len() < MAX_SAMPLES {
            a.group.sample_ids.push(u32::try_from(r.id()).unwrap_or(u32::MAX));
        }
    }

    /// The groups, most frequent first.
    pub fn finish(self) -> Vec<ExceptionGroup> {
        let mut out: Vec<ExceptionGroup> = self
            .accs
            .into_iter()
            .map(|mut a| {
                let mut inst: Vec<(String, usize)> = a.instances.into_iter().collect();
                inst.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
                a.group.instances = inst;
                a.group
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.fingerprint.cmp(&b.fingerprint)));
        out
    }
}
