//! A fake DevTools browser for tests: never a real browser. [`FakeBrowser::install`] writes an executable script that
//! records its command line and re-runs the *test binary* with a marker environment variable; a test function of the
//! caller then calls [`serve_if_requested`], which becomes the fake (a loopback WebSocket server that answers the few
//! commands the driver sends). Unix only: the script is `sh`.
//!
//! The fake exits on `Browser.close` unless its [`Mode`] says to ignore it, so both the graceful close and the
//! kill-after-timeout paths can be tested.

use serde_json::{json, Value};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use tungstenite::Message;

const ARGS_ENV: &str = "FAKE_BROWSER_ARGS_FILE";
const MODE_ENV: &str = "FAKE_BROWSER_MODE";
const TABS_ENV: &str = "FAKE_BROWSER_TABS";

/// How the fake treats `Browser.close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Answers and exits, like a browser that closes.
    Obey,
    /// Answers and keeps running, like a hung browser.
    IgnoreClose,
}

/// An installed fake: the executable to launch and the files it writes.
pub struct FakeBrowser {
    pub exe: PathBuf,
    /// The command line the fake was started with, one argument per line.
    pub args_file: PathBuf,
    /// The address of the page; the fake writes it on `Page.navigate` and a test may overwrite it (the user navigating).
    pub url_file: PathBuf,
}

impl FakeBrowser {
    /// Writes the fake into `dir`. `test_path` is the full path of the caller's test function that calls
    /// [`serve_if_requested`] (for example `browser::renderer::tests::fake_browser_process`); `tabs` is how many pages it reports.
    pub fn install(dir: &Path, test_path: &str, mode: Mode, tabs: usize) -> std::io::Result<FakeBrowser> {
        let me = std::env::current_exe()?;
        let args_file = dir.join("args.txt");
        let url_file = dir.join("url.txt");
        let exe = dir.join("fake-browser.sh");
        let mode = if mode == Mode::Obey { "obey" } else { "ignore" };
        // macOS strips DYLD_* variables when it runs a system shell (SIP): the script puts them back for the test binary.
        let dyld = std::env::var("DYLD_LIBRARY_PATH")
            .map(|v| format!("export DYLD_LIBRARY_PATH='{v}'\n"))
            .unwrap_or_default();
        let script = format!(
            "#!/bin/sh\n{dyld}printf '%s\\n' \"$@\" > '{args}'\nexport {ARGS_ENV}='{args}' {MODE_ENV}={mode} {TABS_ENV}={tabs} FAKE_BROWSER_URL='{url}'\nexec '{me}' '{test_path}' --exact --nocapture\n",
            args = args_file.display(),
            url = url_file.display(),
            me = me.display(),
        );
        std::fs::write(&exe, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(FakeBrowser { exe, args_file, url_file })
    }

    /// The arguments the fake was started with (empty until it started).
    pub fn args(&self) -> Vec<String> {
        std::fs::read_to_string(&self.args_file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

/// Becomes the fake browser when this process was started by [`FakeBrowser::install`]'s script; returns at once otherwise
/// (so the caller's marker test is harmless in a normal run). Never returns when it serves.
pub fn serve_if_requested() {
    let Ok(args_file) = std::env::var(ARGS_ENV) else { return };
    let ignore = std::env::var(MODE_ENV).as_deref() == Ok("ignore");
    let tabs: usize = std::env::var(TABS_ENV).ok().and_then(|t| t.parse().ok()).unwrap_or(1);
    let url_file = std::env::var("FAKE_BROWSER_URL").unwrap_or_default();
    let dir = std::fs::read_to_string(&args_file)
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("--user-data-dir=").map(PathBuf::from)));
    let (Some(dir), Ok(listener)) = (dir, TcpListener::bind("127.0.0.1:0")) else {
        std::process::exit(2)
    };
    let Ok(addr) = listener.local_addr() else { std::process::exit(2) };
    if std::fs::write(dir.join("DevToolsActivePort"), format!("{}\n/devtools/browser/fake\n", addr.port())).is_err() {
        std::process::exit(2);
    }
    for stream in listener.incoming().flatten() {
        let url_file = url_file.clone();
        std::thread::spawn(move || connection(stream, ignore, tabs, &url_file));
    }
    std::process::exit(0);
}

fn connection(stream: std::net::TcpStream, ignore: bool, tabs: usize, url_file: &str) {
    let Ok(mut ws) = tungstenite::accept(stream) else { return };
    while let Ok(msg) = ws.read() {
        let Message::Text(t) = msg else { continue };
        let Ok(req) = serde_json::from_str::<Value>(t.as_str()) else {
            continue;
        };
        let method = req["method"].as_str().unwrap_or("");
        let url = || std::fs::read_to_string(url_file).unwrap_or_else(|_| "about:blank".into());
        let result = match method {
            "Target.createTarget" => json!({"targetId": "T1"}),
            "Target.attachToTarget" => json!({"sessionId": "S1"}),
            "Browser.getWindowForTarget" => json!({"windowId": 1}),
            "Target.getTargets" => {
                json!({"targetInfos": (0..tabs).map(|i| json!({"targetId": format!("T{}", i + 1), "type": "page"})).collect::<Vec<_>>()})
            }
            "Target.getTargetInfo" => json!({"targetInfo": {"url": url(), "title": "fake"}}),
            "Runtime.evaluate" if req["params"]["expression"].as_str() == Some("document.readyState") => {
                json!({"result": {"value": "complete"}})
            }
            "Page.navigate" => {
                if let Some(u) = req["params"]["url"].as_str() {
                    let _ = std::fs::write(url_file, u);
                }
                json!({"frameId": "F"})
            }
            _ => json!({}),
        };
        let mut reply = json!({"id": req["id"], "result": result});
        if let Some(s) = req.get("sessionId") {
            reply["sessionId"] = s.clone();
        }
        if ws.send(Message::text(reply.to_string())).is_err() {
            return;
        }
        if method == "Page.navigate" {
            let ev = json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}});
            let _ = ws.send(Message::text(ev.to_string()));
        }
        if method == "Browser.close" && !ignore {
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
    }
}
