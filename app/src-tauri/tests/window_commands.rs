//! Regression test: Help ▸ Show activity log opened a white window on Windows. The command was synchronous,
//! and a window built from a synchronous command deadlocks WebView2 on Windows (the command holds the main
//! thread the new webview needs to load). Every command that builds a window must be `async`.

use std::path::{Path, PathBuf};

/// Calls that build a window (directly or through Debrief's window helpers).
const WINDOW_BUILDERS: &[&str] = &[
    "open_new(",
    "WebviewWindowBuilder",
    "native_feel::build(",
    "panel_windows::open(",
    "window::open(",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The text of the function that follows `#[tauri::command]` at `start`: its signature and its body
/// (to the brace that closes it).
fn function_after(src: &str, start: usize) -> &str {
    let rest = &src[start..];
    let Some(open) = rest.find('{') else { return rest };
    let mut depth = 0usize;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..open + i + 1];
                }
            }
            _ => {}
        }
    }
    rest
}

/// Commands in `src` that build a window but are not `async`, as `name (file)`.
fn sync_window_commands(file: &str, src: &str) -> Vec<String> {
    // Test modules build windows from plain functions on purpose.
    let src = src.replace("\r\n", "\n");
    let src = src.split("#[cfg(test)]\nmod tests").next().unwrap_or_default();
    let mut bad = Vec::new();
    for (at, _) in src.match_indices("#[tauri::command]") {
        let f = function_after(src, at);
        let signature = f.lines().find(|l| l.contains("fn ")).unwrap_or_default();
        if WINDOW_BUILDERS.iter().any(|b| f.contains(b)) && !signature.contains("async fn") {
            bad.push(format!("{} ({file})", signature.trim()));
        }
    }
    bad
}

#[test]
fn every_command_that_builds_a_window_is_async() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src_dir, &mut files);
    let mut checked = 0;
    let mut bad = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        checked += src.matches("#[tauri::command]").count();
        bad.extend(sync_window_commands(&f.display().to_string(), &src));
    }
    assert!(
        checked > 100,
        "found only {checked} commands; is the scan looking in the right place?"
    );
    assert!(
        bad.is_empty(),
        "these commands build a window but are not async (they deadlock on Windows):\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_scan_flags_a_synchronous_window_command() {
    let sync = "#[tauri::command]\npub fn open_log(app: AppHandle) -> CmdResult<String> {\n    if x { y }\n    crate::window_manager::open_new(&app, vec![])\n}\n";
    assert_eq!(sync_window_commands("x.rs", sync).len(), 1);
    let fine = sync.replace("pub fn", "pub async fn");
    assert!(sync_window_commands("x.rs", &fine).is_empty());
    let unrelated =
        "#[tauri::command]\npub fn list(app: AppHandle) -> Vec<String> {\n    vec![]\n}\nfn helper() { open_new(&app, vec![]) }\n";
    assert!(sync_window_commands("x.rs", unrelated).is_empty());
}
