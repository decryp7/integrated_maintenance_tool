//! "Open file", "Open at line" and "Show in folder" for an opened source. The UI sends a source id
//! (and the event's line); the engine resolves the file from the window's own sources
//! (`debrief_core::file_target`), so no path ever comes from the webview.

use crate::opener::{self, Invocation, Os};
use crate::windows::CurrentWindow;
use crate::CmdResult;
use debrief_core::file_target::{resolve, FileAction, FileTarget, OpenFileRequest};
use serde::Serialize;
use std::path::Path;

/// What opened the file (the UI says so when "Open at line" fell back to the default program).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OpenedWith {
    Editor,
    DefaultApp,
    FileManager,
}

/// `open_source_file` reply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileOpened {
    pub with: OpenedWith,
    /// The file that was opened or shown.
    pub path: String,
}

/// The command line for `action` on `target`: VS Code at the line when `editor` is known, else the
/// default program; never the default program of something it would run.
pub(crate) fn plan(os: Os, action: FileAction, target: &FileTarget, editor: Option<&Path>) -> Result<(OpenedWith, Invocation), String> {
    match (action, target.line, editor) {
        (FileAction::Reveal, ..) => Ok((OpenedWith::FileManager, opener::reveal(os, &target.path))),
        (FileAction::OpenAtLine, Some(line), Some(editor)) => Ok((OpenedWith::Editor, opener::open_at_line(editor, &target.path, line))),
        _ if opener::runs_on_open(&target.path) => Err(format!(
            "Debrief does not open {} with its default program, because that could run it: use Show in folder{}",
            target.name,
            if action == FileAction::OpenAtLine { "" } else { " or Open at line" }
        )),
        _ => Ok((OpenedWith::DefaultApp, opener::open_default(os, &target.path))),
    }
}

/// The file `target` names, canonicalized (links resolved), so the checks below see what the OS would open:
/// a link named `app.log` that points at a script is treated as the script. Fails when it was moved, deleted
/// or replaced by a folder since it was opened.
fn existing_file(target: &FileTarget) -> Result<FileTarget, String> {
    let gone = || {
        format!(
            "{} is no longer at {}: it was moved or deleted",
            target.name,
            opener::plain_path(&target.path).display()
        )
    };
    let path = std::fs::canonicalize(&target.path).map_err(|_| gone())?;
    if !path.is_file() {
        return Err(gone());
    }
    Ok(FileTarget { path, ..target.clone() })
}

/// Checks the file still exists, then starts the helper for `action` (detached, with a timeout). The file is
/// never read here, so a file another program holds locked is still opened or shown.
fn open(action: FileAction, target: &FileTarget) -> Result<FileOpened, String> {
    let target = &existing_file(target)?;
    let editor = (action == FileAction::OpenAtLine).then(opener::installed_vscode).flatten();
    let (with, invocation) = plan(Os::CURRENT, action, target, editor.as_deref())?;
    let path = opener::plain_path(&target.path).to_string_lossy().into_owned();
    opener::start(&invocation).map_err(|e| format!("could not open {} ({e}); it is at {path}", target.name))?;
    log::info!(With = format!("{with:?}").as_str(); "Source file opened outside Debrief");
    Ok(FileOpened { with, path })
}

/// Opens one of the calling window's sources with the default program, in an editor at a line, or in
/// the file manager. Refuses ids that are not (or no longer) open in this window and archive members.
#[tauri::command]
pub(crate) async fn open_source_file(state: CurrentWindow, request: OpenFileRequest) -> CmdResult<FileOpened> {
    let sources = crate::workspace_commands::summary_of(&state).await?.sources;
    let target = resolve(&sources, &request).map_err(|e| e.to_string())?;
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || open(request.action, &target))
            .await
            .map_err(|e| e.to_string())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn target(path: &str, line: Option<u32>) -> FileTarget {
        FileTarget {
            path: PathBuf::from(path),
            line,
            name: "app.log".into(),
        }
    }

    #[test]
    fn open_at_line_uses_the_editor_when_there_is_one() {
        let code = Path::new("/usr/bin/code");
        let (with, i) = plan(Os::Linux, FileAction::OpenAtLine, &target("/l/app.log", Some(9)), Some(code)).unwrap();
        assert_eq!(with, OpenedWith::Editor);
        assert_eq!(i, opener::open_at_line(code, Path::new("/l/app.log"), 9));
        let (with, i) = plan(Os::Windows, FileAction::OpenAtLine, &target(r"C:\l\app.log", Some(9)), None).unwrap();
        assert_eq!((with, i.program.to_str()), (OpenedWith::DefaultApp, Some("explorer")));
    }

    #[test]
    fn open_and_reveal_per_platform() {
        for os in [Os::Windows, Os::Mac, Os::Linux] {
            let t = target("/l/app.log", None);
            assert_eq!(
                plan(os, FileAction::Open, &t, None).unwrap(),
                (OpenedWith::DefaultApp, opener::open_default(os, &t.path))
            );
            assert_eq!(
                plan(os, FileAction::Reveal, &t, None).unwrap(),
                (OpenedWith::FileManager, opener::reveal(os, &t.path))
            );
        }
    }

    #[test]
    fn never_runs_a_script_through_its_default_program() {
        let t = FileTarget {
            name: "deploy.bat".into(),
            ..target("/l/deploy.bat", None)
        };
        let e = plan(Os::Windows, FileAction::Open, &t, None).unwrap_err();
        assert!(e.contains("deploy.bat") && e.contains("Open at line"), "{e}");
        let at_line = FileTarget {
            line: Some(3),
            ..t.clone()
        };
        assert!(plan(Os::Windows, FileAction::OpenAtLine, &at_line, None)
            .unwrap_err()
            .contains("Show in folder"));
        // In an editor or the file manager it is only shown, never run.
        assert_eq!(
            plan(Os::Windows, FileAction::OpenAtLine, &at_line, Some(Path::new("code")))
                .unwrap()
                .0,
            OpenedWith::Editor
        );
        assert_eq!(plan(Os::Windows, FileAction::Reveal, &t, None).unwrap().0, OpenedWith::FileManager);
    }

    #[test]
    fn a_moved_file_is_reported_not_opened() {
        let missing = std::env::temp_dir().join("debrief-no-such-dir").join("gone.log");
        let e = open(FileAction::Open, &target(&missing.to_string_lossy(), None)).unwrap_err();
        assert!(e.contains("moved or deleted") && e.contains("app.log"), "{e}");
    }

    #[test]
    fn an_existing_file_is_handed_to_the_os() {
        let dir = std::env::temp_dir().join(format!("debrief-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.log");
        std::fs::write(&file, "x\n").unwrap();
        let t = target(&file.to_string_lossy(), Some(1));
        assert_eq!(open(FileAction::Reveal, &t).unwrap().with, OpenedWith::FileManager);
        assert_eq!(open(FileAction::Open, &t).unwrap().with, OpenedWith::DefaultApp);
        let at_line = open(FileAction::OpenAtLine, &t).unwrap();
        assert!(matches!(at_line.with, OpenedWith::Editor | OpenedWith::DefaultApp));
        assert_eq!(
            serde_json::to_value(&at_line).unwrap()["path"],
            serde_json::json!(opener::plain_path(&std::fs::canonicalize(&file).unwrap()).to_string_lossy())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_replaced_by_a_folder_is_reported_not_opened() {
        let dir = std::env::temp_dir().join(format!("debrief-open-dir-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app.log")).unwrap();
        let e = open(FileAction::Reveal, &target(&dir.join("app.log").to_string_lossy(), None)).unwrap_err();
        assert!(e.contains("moved or deleted"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_file_is_still_opened_and_shown() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("debrief-open-locked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("locked.log");
        std::fs::write(
            &file, "x
",
        )
        .unwrap();
        // No sharing: another process holds the file exclusively (as a writer that locks its log does).
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        let t = target(&file.to_string_lossy(), Some(1));
        assert_eq!(open(FileAction::Reveal, &t).unwrap().with, OpenedWith::FileManager);
        assert_eq!(open(FileAction::Open, &t).unwrap().with, OpenedWith::DefaultApp);
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_judged_by_the_file_it_points_at() {
        let dir = std::env::temp_dir().join(format!("debrief-open-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("run.sh"),
            "echo hi
",
        )
        .unwrap();
        let link = dir.join("app.log");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("run.sh"), &link).unwrap();
        let e = open(FileAction::Open, &target(&link.to_string_lossy(), None)).unwrap_err();
        assert!(e.contains("could run it"), "{e}");
        assert_eq!(
            open(FileAction::Reveal, &target(&link.to_string_lossy(), None)).unwrap().path,
            std::fs::canonicalize(dir.join("run.sh")).unwrap().to_string_lossy()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_command_resolves_only_this_windows_sources() {
        use crate::test_support::{samples, test_app_with_commands, window, TempDir};
        use crate::tools::tests::ipc;
        use serde_json::json;
        let app = test_app_with_commands(None);
        let main = window(&app, crate::windows::MAIN);
        let dir = TempDir::new("open-source-file");
        crate::paths::allow_under(app.handle(), crate::windows::MAIN, &dir.0);
        let folder = dir.0.clone();
        let archives = samples().join("archives").join("notify01");
        for f in ["app.log.3.gz", "notify01-logs-20261014.zip"] {
            std::fs::copy(archives.join(f), folder.join(f)).unwrap();
        }
        std::fs::write(
            folder.join("plain.log"),
            "2026-10-14 10:00:00 [INF] started\n2026-10-14 10:00:01 [ERR] failed\n",
        )
        .unwrap();
        let summary = ipc(&main, "open_paths", json!({ "paths": [folder.to_string_lossy()] })).unwrap();
        let sources = summary["sources"].as_array().unwrap();
        let by_kind = |kind: &str| {
            sources
                .iter()
                .find(|s| s["fileKind"] == kind)
                .unwrap_or_else(|| panic!("no {kind} source"))["id"]
                .clone()
        };
        let open = |w: &tauri::WebviewWindow<tauri::test::MockRuntime>, body| ipc(w, "open_source_file", json!({ "request": body }));
        let err = |r: Result<serde_json::Value, serde_json::Value>| r.unwrap_err()["message"].as_str().unwrap_or_default().to_string();

        let text = by_kind("text");
        assert_eq!(
            open(&main, json!({ "sourceId": text, "action": "openAtLine", "line": 2 })).unwrap()["path"]
                .as_str()
                .map(|p| p.ends_with(".log")),
            Some(true)
        );
        assert_eq!(
            open(&main, json!({ "sourceId": text, "action": "reveal" })).unwrap()["with"],
            "fileManager"
        );

        let member = by_kind("archiveMember");
        assert!(err(open(&main, json!({ "sourceId": member, "action": "open" }))).contains("inside the archive"));
        assert!(open(&main, json!({ "sourceId": member, "action": "reveal" })).unwrap()["path"]
            .as_str()
            .unwrap()
            .ends_with(".zip"));
        let gz = by_kind("compressed");
        assert!(err(open(&main, json!({ "sourceId": gz, "action": "openAtLine", "line": 1 }))).contains("no line to jump to"));

        assert!(err(open(&main, json!({ "sourceId": 999, "action": "open" }))).contains("no longer open"));
        // Another window has its own sources: the ids of this one mean nothing there.
        let other = window(&app, "w-2");
        assert!(err(open(&other, json!({ "sourceId": text, "action": "open" }))).contains("no longer open"));
        // A removed source is no longer reachable.
        ipc(&main, "remove_source", json!({ "sourceId": text })).unwrap();
        assert!(err(open(&main, json!({ "sourceId": text, "action": "reveal" }))).contains("no longer open"));
        // No path is accepted from the webview.
        assert!(open(
            &main,
            json!({ "sourceId": member, "action": "open", "path": "C:/Windows/notepad.exe" })
        )
        .is_err());
    }
}
