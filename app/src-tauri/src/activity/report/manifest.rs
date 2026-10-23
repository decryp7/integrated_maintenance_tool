//! `manifest.json` of a diagnostics report: what kind of zip it is, its format version, which Debrief made it
//! and when, and every file with its kind. Readers parse it once into [`Manifest`] ([`parse`]): an
//! unknown file kind is tolerated ([`FileKind::Other`]); a newer schema, a different `kind`, too many
//! files or an unsafe file name is refused with a typed [`ManifestError`].

use serde::{Deserialize, Serialize};

/// Name of the manifest inside the zip.
pub const MANIFEST_NAME: &str = "manifest.json";
/// `kind` of a diagnostics report.
pub const MANIFEST_KIND: &str = "debrief-diagnostics";
/// Format version this Debrief writes and reads.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest manifest read.
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// Most files a manifest may list.
pub const MAX_MANIFEST_FILES: usize = 2000;
/// Longest file name in a manifest.
pub const MAX_NAME: usize = 200;

/// What a file of the report is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    /// Debrief's own CLEF log.
    Log,
    /// A child process's log (`logs/children/<name>.log`).
    ChildLog,
    /// The activity entries of Debrief's log.
    Activity,
    SystemInfo,
    Processes,
    Steps,
    Breadcrumbs,
    /// An error snapshot's image.
    Snapshot,
    /// An error snapshot's details (`.json`).
    SnapshotDetails,
    Description,
    SessionRecovery,
    /// A kind this Debrief does not know (written by a later version within the same schema); ignored.
    #[serde(other)]
    Other,
}

impl FileKind {
    /// Logs are imported as sources when the report is opened; nothing else is.
    pub fn is_log(self) -> bool {
        matches!(self, FileKind::Log | FileKind::ChildLog | FileKind::Activity)
    }
}

/// One file listed in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub name: String,
    pub kind: FileKind,
    pub bytes: u64,
}

/// The manifest of a diagnostics report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub kind: String,
    pub schema_version: u32,
    pub debrief_version: String,
    pub commit: String,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub files: Vec<ManifestFile>,
}

impl Manifest {
    pub fn new(debrief_version: String, commit: String, created_at: String, files: Vec<ManifestFile>) -> Manifest {
        Manifest {
            kind: MANIFEST_KIND.into(),
            schema_version: SCHEMA_VERSION,
            debrief_version,
            commit,
            created_at,
            files,
        }
    }

    /// The files of one kind, in manifest order.
    pub fn of_kind(&self, kind: FileKind) -> impl Iterator<Item = &ManifestFile> {
        self.files.iter().filter(move |f| f.kind == kind)
    }
}

/// Why a manifest was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Not JSON, or not a manifest of any kind.
    Malformed(String),
    /// A manifest of something else (a Debrief bundle, another program's zip).
    OtherKind,
    /// Made by a newer Debrief.
    Newer(u32),
    TooLarge,
    TooManyFiles(usize),
    UnsafeName(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Malformed(why) => write!(f, "its manifest is not valid ({why})"),
            ManifestError::OtherKind => write!(f, "it is not a Debrief diagnostics report"),
            ManifestError::Newer(v) => write!(
                f,
                "it was made by a newer Debrief (report format {v}; this Debrief reads format {SCHEMA_VERSION}); update Debrief to open it as a report"
            ),
            ManifestError::TooLarge => write!(f, "its manifest is larger than {} KB", MAX_MANIFEST_BYTES / 1024),
            ManifestError::TooManyFiles(n) => write!(f, "its manifest lists {n} files (at most {MAX_MANIFEST_FILES})"),
            ManifestError::UnsafeName(n) => write!(
                f,
                "its manifest lists an invalid file name ({})",
                n.chars().take(60).collect::<String>()
            ),
        }
    }
}

/// A file name inside a report: relative, `/`-separated segments of `[A-Za-z0-9._-]`, no `..`, at most
/// [`MAX_NAME`] bytes.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.split('/').all(|seg| {
            !seg.is_empty() && seg != "." && seg != ".." && seg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// Parses and validates a manifest.
pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge);
    }
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| ManifestError::Malformed(format!("line {}, column {}", e.line(), e.column())))?;
    if v.get("kind").and_then(|k| k.as_str()) != Some(MANIFEST_KIND) {
        return Err(ManifestError::OtherKind);
    }
    match v.get("schemaVersion").and_then(|s| s.as_u64()) {
        Some(s) if s > u64::from(SCHEMA_VERSION) => return Err(ManifestError::Newer(s.min(u64::from(u32::MAX)) as u32)),
        Some(_) => {}
        None => return Err(ManifestError::Malformed("no schemaVersion".into())),
    }
    if let Some(n) = v.get("files").and_then(|f| f.as_array()).map(Vec::len) {
        if n > MAX_MANIFEST_FILES {
            return Err(ManifestError::TooManyFiles(n));
        }
    }
    let m: Manifest = serde_json::from_value(v).map_err(|e| ManifestError::Malformed(e.to_string().chars().take(120).collect()))?;
    if let Some(bad) = m.files.iter().find(|f| !safe_name(&f.name)) {
        return Err(ManifestError::UnsafeName(bad.name.clone()));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> serde_json::Value {
        json!({
            "kind": "debrief-diagnostics", "schemaVersion": 1, "debriefVersion": "0.1.0", "commit": "abc", "createdAt": "2026-10-14T09:00:00.000Z",
            "files": [{ "name": "logs/debrief.log", "kind": "log", "bytes": 3 }, { "name": "x/y.bin", "kind": "future-kind", "bytes": 1 }],
        })
    }

    #[test]
    fn manifests_round_trip_and_tolerate_unknown_kinds() {
        let m = parse(sample().to_string().as_bytes()).unwrap();
        assert_eq!(m.files[1].kind, FileKind::Other);
        assert_eq!(m.of_kind(FileKind::Log).count(), 1);
        assert!(FileKind::ChildLog.is_log() && !FileKind::SystemInfo.is_log());
        let back = parse(&serde_json::to_vec(&Manifest::new("1".into(), "c".into(), "t".into(), vec![])).unwrap()).unwrap();
        assert_eq!(back.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn bad_manifests_are_refused_with_a_reason() {
        let with = |k: &str, v: serde_json::Value| {
            let mut s = sample();
            s[k] = v;
            parse(s.to_string().as_bytes())
        };
        assert_eq!(with("kind", json!("debrief-bundle")), Err(ManifestError::OtherKind));
        assert_eq!(with("schemaVersion", json!(7)), Err(ManifestError::Newer(7)));
        assert!(matches!(with("schemaVersion", json!("one")), Err(ManifestError::Malformed(_))));
        assert!(matches!(with("files", json!("nope")), Err(ManifestError::Malformed(_))));
        let many: Vec<_> = (0..=MAX_MANIFEST_FILES)
            .map(|i| json!({"name": format!("f{i}"), "kind": "log", "bytes": 0}))
            .collect();
        assert_eq!(with("files", json!(many)), Err(ManifestError::TooManyFiles(MAX_MANIFEST_FILES + 1)));
        for bad in ["../evil.log", "/abs.log", "a\\b.log", "c:/x.log", "a//b", ""] {
            assert!(
                matches!(
                    with("files", json!([{"name": bad, "kind": "log", "bytes": 0}])),
                    Err(ManifestError::UnsafeName(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(parse(b"{ nope"), Err(ManifestError::Malformed(_))));
        assert_eq!(parse(&vec![b' '; MAX_MANIFEST_BYTES as usize + 1]), Err(ManifestError::TooLarge));
        for e in [
            ManifestError::Malformed("x".into()),
            ManifestError::OtherKind,
            ManifestError::Newer(2),
            ManifestError::TooLarge,
            ManifestError::TooManyFiles(3),
            ManifestError::UnsafeName("..".into()),
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn names_are_checked() {
        assert!(safe_name("logs/children/plugin-host-net.decryptology.web.log"));
        assert!(!safe_name(&"a".repeat(MAX_NAME + 1)));
        assert!(!safe_name("logs/./x"));
    }
}
