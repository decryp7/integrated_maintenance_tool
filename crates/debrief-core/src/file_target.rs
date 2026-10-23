//! Where "Open file", "Open at line" and "Show in folder" point: the file on disk behind one of the
//! workspace's sources. The UI names a source by id (never by path); the engine resolves the path from
//! the sources it opened, so the webview cannot make Debrief open an arbitrary file.

use crate::api::{FileKind, SourceInfo};
use crate::archive::ENTRY_SEPARATOR;
use serde::Deserialize;
use std::path::PathBuf;

/// What to do with a source's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileAction {
    /// Open it with the program the OS associates with it.
    Open,
    /// Open it in an editor at the event's line.
    OpenAtLine,
    /// Show it selected in the OS file manager.
    Reveal,
}

/// `open_source_file` request: a source of the calling window's workspace, and the event's line.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenFileRequest {
    pub source_id: u32,
    pub action: FileAction,
    /// The event's line (1-based); required for [`FileAction::OpenAtLine`], ignored otherwise.
    #[serde(default)]
    pub line: Option<u32>,
}

/// The resolved file: an opened source's path (or the archive holding it, for "Show in folder").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub path: PathBuf,
    /// Set only for [`FileAction::OpenAtLine`].
    pub line: Option<u32>,
    /// The source's display name, for messages.
    pub name: String,
}

/// Why a source's file cannot be opened that way. The messages say what to do instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FileTargetError {
    #[error("that file is no longer open in this window")]
    UnknownSource,
    #[error("{name} is inside the archive {archive}: extract it to open it, or use Show in folder")]
    InsideArchive { name: String, archive: String },
    #[error("{name} is not a text file, so there is no line to jump to: use Open file")]
    NoLines { name: String },
    #[error("this event has no line number: use Open file")]
    NoLine,
}

/// Resolves `req` against the workspace's `sources`. Pure: does not touch the file system.
pub fn resolve(sources: &[SourceInfo], req: &OpenFileRequest) -> Result<FileTarget, FileTargetError> {
    let source = sources
        .iter()
        .find(|s| s.id == req.source_id)
        .ok_or(FileTargetError::UnknownSource)?;
    let name = source.name.clone();
    let archive = source.path.split_once(ENTRY_SEPARATOR).map(|(file, _)| file);
    let target = |path: &str, line| FileTarget {
        path: PathBuf::from(path),
        line,
        name: name.clone(),
    };
    match (req.action, source.file_kind, archive) {
        (FileAction::Reveal, _, Some(file)) => Ok(target(file, None)),
        (_, FileKind::ArchiveMember, _) | (_, _, Some(_)) => Err(FileTargetError::InsideArchive {
            archive: archive_name(archive.unwrap_or(&source.path)),
            name,
        }),
        (FileAction::Open | FileAction::Reveal, _, None) => Ok(target(&source.path, None)),
        (FileAction::OpenAtLine, FileKind::Text, None) => match req.line.filter(|l| *l >= 1) {
            Some(line) => Ok(target(&source.path, Some(line))),
            None => Err(FileTargetError::NoLine),
        },
        (FileAction::OpenAtLine, _, None) => Err(FileTargetError::NoLines { name }),
    }
}

fn archive_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u32, path: &str, kind: FileKind) -> SourceInfo {
        SourceInfo {
            id,
            path: path.into(),
            name: format!("name{id}"),
            group: String::new(),
            importer: "text".into(),
            records: 1,
            skipped: 0,
            split_events: 0,
            dropped: 0,
            step_stats: Vec::new(),
            error: None,
            from: None,
            to: None,
            offset_ms: 0,
            file_kind: kind,
            timestamp_issue: None,
            routed_from: None,
            encoding: None,
            problems: Vec::new(),
            identity: Vec::new(),
            parser: None,
        }
    }

    fn req(source_id: u32, action: FileAction, line: Option<u32>) -> OpenFileRequest {
        OpenFileRequest { source_id, action, line }
    }

    fn sources() -> Vec<SourceInfo> {
        vec![
            source(0, "/logs/app.log", FileKind::Text),
            source(1, "/logs/bundle.zip!/inner/a.log", FileKind::ArchiveMember),
            source(2, "/logs/old.log.gz", FileKind::Compressed),
            source(3, "/logs/System.evtx", FileKind::Binary),
        ]
    }

    #[test]
    fn opens_a_text_file_at_the_line() {
        let t = resolve(&sources(), &req(0, FileAction::OpenAtLine, Some(42))).unwrap();
        assert_eq!(
            (t.path, t.line, t.name.as_str()),
            (PathBuf::from("/logs/app.log"), Some(42), "name0")
        );
        let t = resolve(&sources(), &req(0, FileAction::Open, Some(42))).unwrap();
        assert_eq!(t.line, None, "the line only matters for Open at line");
    }

    #[test]
    fn refuses_unknown_and_removed_sources() {
        assert_eq!(
            resolve(&sources(), &req(9, FileAction::Open, None)),
            Err(FileTargetError::UnknownSource)
        );
        let mut removed = sources();
        removed.retain(|s| s.id != 0);
        assert_eq!(
            resolve(&removed, &req(0, FileAction::Reveal, None)),
            Err(FileTargetError::UnknownSource)
        );
        assert_eq!(
            resolve(&[], &req(0, FileAction::OpenAtLine, Some(1))),
            Err(FileTargetError::UnknownSource)
        );
    }

    #[test]
    fn archive_members_only_reveal_their_archive() {
        for action in [FileAction::Open, FileAction::OpenAtLine] {
            let e = resolve(&sources(), &req(1, action, Some(3))).unwrap_err();
            assert_eq!(
                e,
                FileTargetError::InsideArchive {
                    name: "name1".into(),
                    archive: "bundle.zip".into()
                }
            );
            assert!(e.to_string().contains("extract it"), "{e}");
        }
        let t = resolve(&sources(), &req(1, FileAction::Reveal, None)).unwrap();
        assert_eq!(t.path, PathBuf::from("/logs/bundle.zip"));
        // An archive member is recognised by its path even if its kind says otherwise.
        let odd = vec![source(5, "/x.zip!/a.log", FileKind::Text)];
        assert!(matches!(
            resolve(&odd, &req(5, FileAction::Open, None)),
            Err(FileTargetError::InsideArchive { .. })
        ));
        let odd = vec![source(6, "/weird", FileKind::ArchiveMember)];
        assert!(
            matches!(resolve(&odd, &req(6, FileAction::Reveal, None)), Err(FileTargetError::InsideArchive { archive, .. }) if archive == "weird")
        );
    }

    #[test]
    fn only_text_files_open_at_a_line() {
        for id in [2, 3] {
            assert!(matches!(
                resolve(&sources(), &req(id, FileAction::OpenAtLine, Some(5))),
                Err(FileTargetError::NoLines { .. })
            ));
            assert!(resolve(&sources(), &req(id, FileAction::Open, None)).is_ok());
            assert!(resolve(&sources(), &req(id, FileAction::Reveal, None)).is_ok());
        }
        assert_eq!(
            resolve(&sources(), &req(0, FileAction::OpenAtLine, None)),
            Err(FileTargetError::NoLine)
        );
        assert_eq!(
            resolve(&sources(), &req(0, FileAction::OpenAtLine, Some(0))),
            Err(FileTargetError::NoLine)
        );
    }

    #[test]
    fn requests_parse_strictly() {
        let r: OpenFileRequest = serde_json::from_str(r#"{"sourceId":3,"action":"openAtLine","line":7}"#).unwrap();
        assert_eq!((r.source_id, r.action, r.line), (3, FileAction::OpenAtLine, Some(7)));
        let r: OpenFileRequest = serde_json::from_str(r#"{"sourceId":3,"action":"reveal"}"#).unwrap();
        assert_eq!(r.line, None);
        assert!(serde_json::from_str::<OpenFileRequest>(r#"{"sourceId":3,"action":"run"}"#).is_err());
        assert!(serde_json::from_str::<OpenFileRequest>(r#"{"sourceId":3,"action":"open","path":"/etc/passwd"}"#).is_err());
        assert!(serde_json::from_str::<OpenFileRequest>(r#"{"sourceId":-1,"action":"open"}"#).is_err());
    }
}
