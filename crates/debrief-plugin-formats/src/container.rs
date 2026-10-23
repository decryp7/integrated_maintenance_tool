//! Shared by the Docker and CRI importers: the output stream, the level
//! heuristic, metadata from the standard log paths and partial-line joining.

use debrief_plugin_api::model::{Fields, Level, Record};
use debrief_plugin_api::text::MAX_LINE_LEN;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

/// The stream a container wrote a line to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub fn parse(s: &str) -> Option<Stream> {
        match s {
            "stdout" => Some(Stream::Stdout),
            "stderr" => Some(Stream::Stderr),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        }
    }

    fn index(self) -> usize {
        match self {
            Stream::Stdout => 0,
            Stream::Stderr => 1,
        }
    }
}

/// The level the message states itself, else stderr Warning, stdout Information.
pub(crate) fn level_for(stream: Stream, message: &str) -> Level {
    level_in_message(message).unwrap_or(match stream {
        Stream::Stdout => Level::Information,
        Stream::Stderr => Level::Warning,
    })
}

/// Looks at the start of a message for a level: a JSON `"level"` key, logfmt
/// `level=`, a klog prefix (`E1017 12:00:00.000000`) or an upper-case level word.
pub(crate) fn level_in_message(message: &str) -> Option<Level> {
    static JSON: OnceLock<Regex> = OnceLock::new();
    static LOGFMT: OnceLock<Regex> = OnceLock::new();
    static KLOG: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let head = prefix(message, 256);
    let json = JSON.get_or_init(|| Regex::new(r#""(?:level|severity|lvl|@l)"\s*:\s*"([A-Za-z]{1,12})""#).expect("static regex"));
    let logfmt = LOGFMT.get_or_init(|| Regex::new(r"(?:^|\s)(?:level|lvl|severity)=([A-Za-z]{1,12})\b").expect("static regex"));
    let klog = KLOG.get_or_init(|| Regex::new(r"^([IWEF])\d{4} \d{2}:\d{2}:\d{2}").expect("static regex"));
    let word = WORD.get_or_init(|| Regex::new(r"\b(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL|PANIC)\b").expect("static regex"));
    if let Some(c) = json.captures(head).or_else(|| logfmt.captures(head)) {
        return Level::parse(&c[1]);
    }
    // A Go runtime panic.
    if head.starts_with("panic: ") {
        return Some(Level::Fatal);
    }
    if let Some(c) = klog.captures(head) {
        return Some(match &c[1] {
            "I" => Level::Information,
            "W" => Level::Warning,
            "E" => Level::Error,
            _ => Level::Fatal,
        });
    }
    // Only near the start, where loggers put the level, not anywhere in the text.
    word.captures(prefix(head, 64)).and_then(|c| Level::parse(&c[1]))
}

/// At most `n` bytes of `s`, cut at a character boundary.
fn prefix(s: &str, n: usize) -> &str {
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Kubernetes and Docker metadata from the file's path:
///
/// * `/var/log/pods/<namespace>_<pod>_<uid>/<container>/<restart>.log`
/// * `/var/log/containers/<pod>_<namespace>_<container>-<id>.log`
/// * `/var/lib/docker/containers/<id>/<id>-json.log`
pub(crate) fn path_fields(path: &Path) -> Fields {
    static CONTAINERS: OnceLock<Regex> = OnceLock::new();
    let mut f = Fields::new();
    let text = path.to_string_lossy();
    let parts: Vec<&str> = text.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some((file, dirs)) = parts.split_last() else { return f };
    let n = dirs.len();
    // .../pods/<ns>_<pod>_<uid>/<container>/<N>.log[.rotation]
    if n >= 3 && dirs[n - 3] == "pods" {
        let mut ids = dirs[n - 2].splitn(3, '_');
        let restart = file.split_once(".log").and_then(|(num, _)| num.parse::<u64>().ok());
        if let (Some(ns), Some(pod), Some(uid), Some(restart)) = (ids.next(), ids.next(), ids.next(), restart) {
            if !ns.is_empty() && !pod.is_empty() && !uid.is_empty() {
                f.insert("Namespace".into(), Value::from(ns));
                f.insert("Pod".into(), Value::from(pod));
                f.insert("PodUid".into(), Value::from(uid));
                f.insert("Container".into(), Value::from(dirs[n - 1]));
                f.insert("Restart".into(), Value::from(restart));
                return f;
            }
        }
    }
    if n >= 1 && dirs[n - 1] == "containers" {
        let re = CONTAINERS.get_or_init(|| {
            Regex::new(r"^([a-z0-9][a-z0-9.-]*)_([a-z0-9][a-z0-9-]*)_([A-Za-z0-9._-]+)-([0-9a-f]{64})\.log$").expect("static regex")
        });
        if let Some(c) = re.captures(file) {
            f.insert("Pod".into(), Value::from(&c[1]));
            f.insert("Namespace".into(), Value::from(&c[2]));
            f.insert("Container".into(), Value::from(&c[3]));
            f.insert("ContainerId".into(), Value::from(&c[4]));
            return f;
        }
    }
    // .../containers/<id>/<id>-json.log[.N]
    if n >= 2 && dirs[n - 2] == "containers" && is_container_id(dirs[n - 1]) && file.starts_with(&format!("{}-json.log", dirs[n - 1])) {
        f.insert("ContainerId".into(), Value::from(dirs[n - 1]));
    }
    f
}

fn is_container_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// One message being assembled from partial lines.
struct Pending {
    line: u32,
    ts: Option<i64>,
    text: String,
    fields: Fields,
}

/// Joins partial lines per stream (both runtimes split long lines, at 16 KiB
/// for Docker, and stdout and stderr chunks may interleave). A message grows
/// to at most [`MAX_LINE_LEN`]; then what is there becomes a record and the
/// rest starts the next one.
pub(crate) struct Joiner {
    pending: [Option<Pending>; 2],
    base: Fields,
    pub records: Vec<Record>,
}

impl Joiner {
    /// `base` fields (from the path) go on every record.
    pub fn new(base: Fields) -> Self {
        Joiner {
            pending: [None, None],
            base,
            records: Vec::new(),
        }
    }

    /// Adds one chunk; `last` says it ends the message.
    pub fn push(&mut self, stream: Stream, line: u32, ts: Option<i64>, chunk: &str, last: bool, fields: Fields) {
        let slot = stream.index();
        if self.pending[slot]
            .as_ref()
            .is_some_and(|p| p.text.len() + chunk.len() > MAX_LINE_LEN)
        {
            self.flush(stream);
        }
        match &mut self.pending[slot] {
            Some(p) => p.text.push_str(chunk),
            None => {
                self.pending[slot] = Some(Pending {
                    line,
                    ts,
                    text: chunk.to_string(),
                    fields,
                })
            }
        }
        if last {
            self.flush(stream);
        }
    }

    fn flush(&mut self, stream: Stream) {
        let Some(p) = self.pending[stream.index()].take() else { return };
        let mut r = Record::new(p.line);
        r.ts = p.ts;
        r.level = level_for(stream, &p.text);
        r.fields = self.base.clone();
        r.fields.insert("Stream".into(), Value::from(stream.as_str()));
        r.fields.extend(p.fields);
        r.message = p.text;
        self.records.push(r);
    }

    /// Ends both streams (an unfinished partial line becomes a record) and
    /// returns the records in file order.
    pub fn finish(mut self) -> Vec<Record> {
        self.flush(Stream::Stdout);
        self.flush(Stream::Stderr);
        self.records.sort_by_key(|r| r.line);
        self.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_from_messages_and_streams() {
        let cases = [
            (r#"{"level":"error","msg":"x"}"#, Some(Level::Error)),
            (r#"{"@l": "Warning"}"#, Some(Level::Warning)),
            ("ts=1 level=debug msg=x", Some(Level::Debug)),
            ("level=nonsense", None),
            ("E1017 12:00:00.000000       1 main.go:10] boom", Some(Level::Error)),
            ("W1017 12:00:00.000000 x", Some(Level::Warning)),
            ("I1017 12:00:00.000000 x", Some(Level::Information)),
            ("F1017 12:00:00.000000 x", Some(Level::Fatal)),
            ("2026-10-17 12:00:00 WARN low disk", Some(Level::Warning)),
            ("[ERROR] failed", Some(Level::Error)),
            ("panic: runtime error", Some(Level::Fatal)),
            ("an error happened", None),
            ("plain", None),
        ];
        for (msg, want) in cases {
            assert_eq!(level_in_message(msg), want, "{msg}");
        }
        let late = format!("{} ERROR", "x".repeat(80));
        assert_eq!(level_in_message(&late), None);
        assert_eq!(level_for(Stream::Stderr, "plain"), Level::Warning);
        assert_eq!(level_for(Stream::Stdout, "plain"), Level::Information);
        assert_eq!(level_for(Stream::Stderr, "INFO ok"), Level::Information);
        assert_eq!(prefix("héllo", 2), "h");
        assert_eq!(level_in_message(&"é".repeat(200)), None);
        assert_eq!(Stream::parse("stdin"), None);
    }

    #[test]
    fn metadata_from_standard_paths() {
        let f = path_fields(Path::new("/var/log/pods/shop_orders-7d9f-abcde_0f3c-11ee/orders/2.log"));
        assert_eq!(f["Namespace"], "shop");
        assert_eq!(f["Pod"], "orders-7d9f-abcde");
        assert_eq!(f["PodUid"], "0f3c-11ee");
        assert_eq!(f["Container"], "orders");
        assert_eq!(f["Restart"], 2);
        let rotated = path_fields(Path::new(r"C:\logs\pods\ns_pod_uid\c\0.log.20261017-120000"));
        assert_eq!(rotated["Restart"], 0);
        assert!(path_fields(Path::new("/var/log/pods/bad/c/0.log")).is_empty());
        assert!(path_fields(Path::new("/var/log/pods/a_b_c/c/x.log")).is_empty());
        assert!(path_fields(Path::new("/var/log/pods/_b_c/c/0.log")).is_empty());

        let id = "a".repeat(64);
        let f = path_fields(Path::new(&format!("/var/log/containers/orders-7d9f_shop_orders-{id}.log")));
        assert_eq!(
            (f["Pod"].as_str(), f["Namespace"].as_str(), f["Container"].as_str()),
            (Some("orders-7d9f"), Some("shop"), Some("orders"))
        );
        assert_eq!(f["ContainerId"], id.as_str());
        assert!(path_fields(Path::new("/var/log/containers/whatever.log")).is_empty());

        let f = path_fields(Path::new(&format!("/var/lib/docker/containers/{id}/{id}-json.log.1")));
        assert_eq!(f["ContainerId"], id.as_str());
        assert!(path_fields(Path::new(&format!("/var/lib/docker/containers/{id}/other.log"))).is_empty());
        assert!(path_fields(Path::new("/x/containers/ABC/ABC-json.log")).is_empty());
        assert!(path_fields(Path::new("")).is_empty());
        assert!(path_fields(Path::new("x.log")).is_empty());
    }

    #[test]
    fn joiner_joins_interleaved_partials_and_bounds_size() {
        let mut j = Joiner::new(Fields::new());
        j.push(Stream::Stdout, 1, Some(1), "hel", false, Fields::new());
        j.push(Stream::Stderr, 2, Some(2), "ERROR boom", true, Fields::new());
        j.push(Stream::Stdout, 3, Some(3), "lo", true, Fields::new());
        j.push(Stream::Stdout, 4, Some(4), "dangling", false, Fields::new());
        let r = j.finish();
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].message.as_str(), r[0].ts, r[0].line), ("hello", Some(1), 1));
        assert_eq!((r[1].message.as_str(), r[1].level), ("ERROR boom", Level::Error));
        assert_eq!(r[1].fields["Stream"], "stderr");
        assert_eq!(r[2].message, "dangling");

        let mut j = Joiner::new(Fields::new());
        let big = "x".repeat(MAX_LINE_LEN / 2 + 1);
        for line in 1..=3 {
            j.push(Stream::Stdout, line, None, &big, false, Fields::new());
        }
        let r = j.finish();
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|r| r.message.len() <= MAX_LINE_LEN));
    }
}
