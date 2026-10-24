//! A fake DevTools browser for tests: never a real browser. [`FakeBrowser::install`] writes an executable script that
//! records its command line and re-runs the *test binary* with a marker environment variable; a test function of the
//! caller then calls [`serve_if_requested`], which becomes the fake (a loopback WebSocket server that answers the few
//! commands the driver sends). Unix only: the script is `sh`.
//!
//! The fake exits on `Browser.close` unless its [`Mode`] says to ignore it, so both the graceful close and the
//! kill-after-timeout paths can be tested. A test plays the user with [`FakeBrowser::act`]: closing the window (the
//! page target goes away but the process stays, as Chrome does on macOS), quitting the browser, or losing the
//! DevTools connection.

use serde_json::{json, Value};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tungstenite::Message;

const ARGS_ENV: &str = "FAKE_BROWSER_ARGS_FILE";
const MODE_ENV: &str = "FAKE_BROWSER_MODE";
const TABS_ENV: &str = "FAKE_BROWSER_TABS";
const CTL_ENV: &str = "FAKE_BROWSER_CTL";
const CALLS_ENV: &str = "FAKE_BROWSER_CALLS";
/// How often the fake looks at its control file and at its sockets.
const TICK: Duration = Duration::from_millis(10);

/// What the user does to the fake browser ([`FakeBrowser::act`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAction {
    /// Closes the browser window: the page target is destroyed (its session detaches) and the process keeps running,
    /// as Chrome does on macOS when its last window is closed.
    CloseWindow,
    /// Quits the browser: the process exits.
    Quit,
    /// The DevTools connection drops while the process keeps running.
    DropConnection,
    /// Page loads stop finishing (a page that never fires its load event).
    StallLoads,
}

impl UserAction {
    fn word(self) -> &'static str {
        match self {
            UserAction::CloseWindow => "close-window",
            UserAction::Quit => "quit",
            UserAction::DropConnection => "drop-connection",
            UserAction::StallLoads => "stall-loads",
        }
    }
}

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
    /// The user's actions, one word per line, read by the running fake.
    pub ctl_file: PathBuf,
    /// The DevTools methods the fake was sent, one per line (`Browser.close`, `Target.createTarget`, ...).
    pub calls_file: PathBuf,
}

impl FakeBrowser {
    /// Writes the fake into `dir`. `test_path` is the full path of the caller's test function that calls
    /// [`serve_if_requested`] (for example `browser::renderer::tests::fake_browser_process`); `tabs` is how many pages
    /// the browser has when it starts (its start page, plus any the user opened: 0 for a browser without a window).
    /// As in Chrome, every `Target.createTarget` adds a page to these.
    pub fn install(dir: &Path, test_path: &str, mode: Mode, tabs: usize) -> std::io::Result<FakeBrowser> {
        let me = std::env::current_exe()?;
        let args_file = dir.join("args.txt");
        let url_file = dir.join("url.txt");
        let ctl_file = dir.join("ctl.txt");
        let calls_file = dir.join("calls.txt");
        let exe = dir.join("fake-browser.sh");
        let mode = if mode == Mode::Obey { "obey" } else { "ignore" };
        // macOS strips DYLD_* variables when it runs a system shell (SIP): the script puts them back for the test binary.
        let dyld = std::env::var("DYLD_LIBRARY_PATH")
            .map(|v| format!("export DYLD_LIBRARY_PATH='{v}'\n"))
            .unwrap_or_default();
        let script = format!(
            "#!/bin/sh\n{dyld}printf '%s\\n' \"$@\" > '{args}'\nexport {ARGS_ENV}='{args}' {MODE_ENV}={mode} {TABS_ENV}={tabs} FAKE_BROWSER_URL='{url}' {CTL_ENV}='{ctl}' {CALLS_ENV}='{calls}'\nexec '{me}' '{test_path}' --exact --nocapture\n",
            args = args_file.display(),
            url = url_file.display(),
            ctl = ctl_file.display(),
            calls = calls_file.display(),
            me = me.display(),
        );
        std::fs::write(&exe, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(FakeBrowser {
            exe,
            args_file,
            url_file,
            ctl_file,
            calls_file,
        })
    }

    /// Plays the user: the running fake acts on it within a few milliseconds.
    pub fn act(&self, action: UserAction) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.ctl_file)?;
        writeln!(f, "{}", action.word())
    }

    /// The DevTools methods the fake was sent so far, in order.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.calls_file)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
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
    apply_control();
    // An idle fake still quits when told to.
    std::thread::spawn(|| loop {
        apply_control();
        std::thread::sleep(TICK);
    });
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

/// The user's actions so far (the fake is its own process, so process-wide flags are its state).
static WINDOW_CLOSED: AtomicBool = AtomicBool::new(false);
static CONNECTION_DROPPED: AtomicBool = AtomicBool::new(false);
static LOADS_STALLED: AtomicBool = AtomicBool::new(false);
/// Pages opened over DevTools (`Target.createTarget`), besides the ones the browser started with.
static CREATED: AtomicUsize = AtomicUsize::new(0);

/// Notes a method the fake was sent (best effort: a test that reads it waits for what it expects).
fn record_call(method: &str) {
    let Ok(path) = std::env::var(CALLS_ENV) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{method}");
    }
}

/// How much of the control file was there when this fake started: earlier actions were for an earlier browser.
static CONTROL_START: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Applies the user's actions written since this fake started. Called before every answer too, so an action written
/// before a request is always seen by it.
fn apply_control() {
    let Ok(ctl) = std::env::var(CTL_ENV) else { return };
    let words = std::fs::read_to_string(ctl).unwrap_or_default();
    let start = *CONTROL_START.get_or_init(|| words.len());
    for w in words.get(start..).unwrap_or("").lines() {
        match w.trim() {
            "close-window" => WINDOW_CLOSED.store(true, Ordering::SeqCst),
            "drop-connection" => CONNECTION_DROPPED.store(true, Ordering::SeqCst),
            "stall-loads" => LOADS_STALLED.store(true, Ordering::SeqCst),
            "quit" => std::process::exit(0),
            _ => {}
        }
    }
}

fn connection(stream: std::net::TcpStream, ignore: bool, tabs: usize, url_file: &str) {
    let _ = stream.set_read_timeout(Some(TICK));
    let Ok(mut ws) = tungstenite::accept(stream) else { return };
    let mut told_closed = false;
    // Applies the user's actions; false when the connection is to end. The browser reports a closed window once, as
    // Chrome does: the page's session detaches and its target is destroyed.
    let mut user_acts = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>| -> bool {
        apply_control();
        if CONNECTION_DROPPED.load(Ordering::SeqCst) {
            return false;
        }
        if WINDOW_CLOSED.load(Ordering::SeqCst) && !told_closed {
            told_closed = true;
            for ev in [
                json!({"method": "Target.detachedFromTarget", "params": {"sessionId": "S1", "targetId": "T1"}}),
                json!({"method": "Target.targetDestroyed", "params": {"targetId": "T1"}}),
            ] {
                if ws.send(Message::text(ev.to_string())).is_err() {
                    return false;
                }
            }
        }
        true
    };
    loop {
        if !user_acts(&mut ws) {
            return;
        }
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => return,
        };
        let Message::Text(t) = msg else { continue };
        if !user_acts(&mut ws) {
            return;
        }
        let closed = WINDOW_CLOSED.load(Ordering::SeqCst);
        let Ok(req) = serde_json::from_str::<Value>(t.as_str()) else {
            continue;
        };
        let method = req["method"].as_str().unwrap_or("");
        record_call(method);
        let url = || std::fs::read_to_string(url_file).unwrap_or_else(|_| "about:blank".into());
        let on_page = req.get("sessionId").is_some() || method == "Target.getTargetInfo";
        let mut reply = if closed && on_page {
            // The window is gone: its page answers no more, as Chrome answers for a closed target.
            json!({"id": req["id"], "error": {"code": -32001, "message": "No session or target with given id found"}})
        } else {
            let result = match method {
                "Target.createTarget" => {
                    let n = tabs + CREATED.fetch_add(1, Ordering::SeqCst) + 1;
                    json!({"targetId": format!("T{n}")})
                }
                "Target.attachToTarget" => json!({"sessionId": "S1"}),
                "Browser.getWindowForTarget" => json!({"windowId": 1}),
                "Target.getTargets" => {
                    // The start page(s) first, then the pages opened over DevTools; a closed window has none.
                    let open = if closed { 0 } else { tabs + CREATED.load(Ordering::SeqCst) };
                    json!({"targetInfos": (0..open).map(|i| json!({"targetId": format!("T{}", i + 1), "type": "page", "url": "about:blank"})).collect::<Vec<_>>()})
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
            json!({"id": req["id"], "result": result})
        };
        if let Some(s) = req.get("sessionId") {
            reply["sessionId"] = s.clone();
        }
        if ws.send(Message::text(reply.to_string())).is_err() {
            return;
        }
        if method == "Page.navigate" && !closed && !LOADS_STALLED.load(Ordering::SeqCst) {
            let ev = json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}});
            let _ = ws.send(Message::text(ev.to_string()));
        }
        if method == "Browser.close" && !ignore {
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
    }
}
