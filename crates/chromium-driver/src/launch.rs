//! Starting a browser: the command line, the profile (throwaway or persistent) with an accent theme, and the
//! supervised process. The real user agent and the automation banner are kept on purpose: no stealth.

use crate::cdp::Connection;
use crate::discover::Browser;
use crate::page::Page;
use crate::process::Supervised;
use crate::{Error, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Profile {
    /// A fresh directory, deleted when the browser ends.
    Throwaway,
    /// A per-purpose directory that keeps cookies between runs.
    Persistent(PathBuf),
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub profile: Profile,
    /// Window frame colour as `0xRRGGBB` (best effort; see [`theme_preferences`]).
    pub accent: Option<u32>,
    /// The browser is ended after this long, whatever it does.
    pub max_lifetime: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        LaunchOptions {
            profile: Profile::Throwaway,
            accent: None,
            max_lifetime: Duration::from_secs(30 * 60),
        }
    }
}

/// The command-line arguments. `--enable-automation` keeps Chrome's "controlled by automated test software"
/// banner and `navigator.webdriver = true`; nothing hides either. The window opens on screen like any other:
/// no off-screen position and no headless mode.
pub fn args(data_dir: &Path) -> Vec<String> {
    let mut a = vec![
        "--remote-debugging-port=0".to_string(),
        format!("--user-data-dir={}", data_dir.display()),
        "--enable-automation".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-sync".into(),
        "--disable-extensions".into(),
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--window-size=1100,800".into(),
    ];
    a.push("about:blank".into());
    a
}

/// Browser theme preferences that colour the window frame. Chrome and Edge read the user colour from
/// `browser.theme.user_color2` (ARGB integer) with a scheme and variant; the older `user_color` is written too.
/// Best effort: a version that ignores them still shows the other indicators.
pub fn theme_preferences(accent: u32) -> Value {
    let argb = (0xFF00_0000u32 | (accent & 0x00FF_FFFF)) as i32;
    json!({"browser": {"theme": {
        "user_color": argb, "user_color2": argb,
        "color_variant": 2, "color_variant2": 2,
        "color_scheme": 2, "color_scheme2": 2,
        "is_grayscale": false, "is_grayscale2": false
    }}})
}

/// Writes (merging into any existing file) the theme into `<data_dir>/Default/Preferences`.
pub fn write_theme(data_dir: &Path, accent: u32) -> std::io::Result<()> {
    let dir = data_dir.join("Default");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("Preferences");
    let mut root: Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    merge(&mut root, theme_preferences(accent));
    std::fs::write(file, root.to_string())
}

fn merge(into: &mut Value, from: Value) {
    match (into, from) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}

/// Why a launched browser can no longer be used. Each is final: the browser is ended and a new one started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    /// The process ended (the user quit the browser, or it crashed).
    Exited,
    /// The DevTools connection ended while the process still runs.
    Disconnected,
    /// The page Debrief drives was closed (the user closed its window or tab); on macOS the browser keeps running
    /// without a window.
    PageClosed,
}

/// A running browser. Dropping it ends the process and removes a throwaway profile.
pub struct Chromium {
    // Field order matters: the connection closes before the process is killed and the profile removed.
    conn: Arc<Connection>,
    process: Supervised,
    dir: PathBuf,
    throwaway: bool,
    alive: Arc<AtomicBool>,
    deadline: Instant,
    pub exe: Browser,
}

/// A profile folder no other launch uses: the clock alone is not enough (macOS reads it in microseconds, so two
/// launches at once got the same folder and one browser's port file was read by the other).
fn unique_dir() -> PathBuf {
    static LAUNCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let k = LAUNCHES.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("chromium-driver-{}-{n}-{k}", std::process::id()))
}

impl Chromium {
    pub fn launch(exe: &Browser, opts: &LaunchOptions) -> Result<Chromium> {
        let (dir, throwaway) = match &opts.profile {
            Profile::Throwaway => (unique_dir(), true),
            Profile::Persistent(p) => (p.clone(), false),
        };
        std::fs::create_dir_all(&dir).map_err(|e| Error::Launch(format!("the profile folder could not be created: {e}")))?;
        let _ = std::fs::remove_file(dir.join("DevToolsActivePort"));
        if let Some(c) = opts.accent {
            if let Err(e) = write_theme(&dir, c) {
                log::warn!("The browser frame colour could not be written: {e}");
            }
        }
        let mut cmd = std::process::Command::new(&exe.path);
        cmd.args(args(&dir))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut process = match Supervised::spawn(cmd) {
            Ok(p) => p,
            Err(e) => {
                cleanup(&dir, throwaway);
                return Err(Error::Launch(e.to_string()));
            }
        };
        match connect(&dir, &mut process) {
            Ok(conn) => {
                let alive = Arc::new(AtomicBool::new(true));
                let c = Chromium {
                    conn: Arc::new(conn),
                    process,
                    dir,
                    throwaway,
                    alive: alive.clone(),
                    deadline: Instant::now() + opts.max_lifetime,
                    exe: exe.clone(),
                };
                c.watchdog(opts.max_lifetime);
                Ok(c)
            }
            Err(e) => {
                process.kill();
                cleanup(&dir, throwaway);
                Err(e)
            }
        }
    }

    /// A watchdog that ends the browser when its time is up, even if nobody calls it.
    fn watchdog(&self, max: Duration) {
        let (alive, pid) = (self.alive.clone(), self.process.id());
        let end = self.deadline;
        let _ = std::thread::Builder::new().name("chromium-watchdog".into()).spawn(move || {
            while alive.load(Ordering::Acquire) {
                if Instant::now() >= end {
                    log::warn!("The browser reached its time limit ({max:?}) and is ended");
                    kill_pid(pid);
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.id()
    }

    pub fn is_running(&mut self) -> bool {
        self.process.is_running() && !self.conn.is_closed()
    }

    /// Why the browser and its `page` can no longer be used, if so. No round trip to the browser (see
    /// [`Page::exists`] for one).
    pub fn gone(&mut self, page: &Page) -> Option<Gone> {
        if !self.process.is_running() {
            Some(Gone::Exited)
        } else if self.conn.is_closed() {
            Some(Gone::Disconnected)
        } else {
            page.is_closed().then_some(Gone::PageClosed)
        }
    }

    pub fn profile_dir(&self) -> &Path {
        &self.dir
    }

    /// Opens a page (tab) to drive.
    pub fn new_page(&self) -> Result<Page> {
        Page::attach(self.conn.clone())
    }

    /// The number of open pages (tabs and windows) of the browser; a page the user opened shows up here.
    pub fn page_count(&self) -> Result<usize> {
        let r = self.conn.call(None, "Target.getTargets", json!({}), Duration::from_secs(5))?;
        Ok(r.get("targetInfos").and_then(Value::as_array).map_or(0, |t| {
            t.iter().filter(|i| i.get("type").and_then(Value::as_str) == Some("page")).count()
        }))
    }

    /// Closes the browser the way a user would (`Browser.close`), waits up to `grace` for it to exit, ends it by force
    /// when it does not, and removes a throwaway profile. Returns true when it exited by itself.
    pub fn close_gracefully(&mut self, grace: Duration) -> bool {
        self.alive.store(false, Ordering::Release);
        let _ = self.conn.call(None, "Browser.close", json!({}), grace.min(Duration::from_secs(2)));
        let end = Instant::now() + grace;
        let mut exited = !self.process.is_running();
        while !exited && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(25));
            exited = !self.process.is_running();
        }
        // Whatever is left of the process tree (helpers, or the browser itself when it ignored the request) ends now.
        self.process.kill();
        cleanup(&self.dir, self.throwaway);
        exited
    }

    /// Ends the browser at once.
    pub fn kill(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.process.kill();
    }
}

impl Drop for Chromium {
    fn drop(&mut self) {
        self.kill();
        cleanup(&self.dir, self.throwaway);
    }
}

fn kill_pid(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(p) = pid {
        // SAFETY: the pid is our own child's group leader; a reused pid is guarded by the alive flag.
        unsafe { libc::killpg(p as libc::pid_t, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn cleanup(dir: &Path, throwaway: bool) {
    if throwaway {
        // Chrome may still be flushing for a moment after it was killed.
        for _ in 0..5 {
            if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Waits for the `DevToolsActivePort` file Chrome writes into the profile and connects to the browser endpoint.
fn connect(dir: &Path, process: &mut Supervised) -> Result<Connection> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(s) = std::fs::read_to_string(dir.join("DevToolsActivePort")) {
            let mut l = s.lines();
            if let (Some(port), Some(path)) = (l.next().and_then(|p| p.trim().parse::<u16>().ok()), l.next()) {
                return Connection::connect(port, path.trim());
            }
        }
        if !process.is_running() {
            return Err(Error::Launch("the browser exited right after it started".into()));
        }
        if Instant::now() > end {
            return Err(Error::Timeout("the browser start"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_launch_gets_its_own_profile_folder() {
        let dirs: std::collections::HashSet<PathBuf> = (0..1000).map(|_| unique_dir()).collect();
        assert_eq!(dirs.len(), 1000);
    }

    #[test]
    fn keeps_the_automation_banner_and_hides_nothing() {
        let a = args(Path::new("/tmp/p"));
        assert!(a.contains(&"--enable-automation".to_string()));
        assert!(a
            .iter()
            .all(|x| !x.contains("AutomationControlled") && !x.contains("user-agent") && !x.contains("headless")));
        assert!(a.contains(&"--user-data-dir=/tmp/p".to_string()));
    }

    #[test]
    fn the_window_opens_on_screen() {
        assert!(!args(Path::new("/p"))
            .iter()
            .any(|x| x.starts_with("--window-position") || x.contains("headless") || x.contains("offscreen")));
    }

    #[test]
    fn theme_is_written_and_merged() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Default")).unwrap();
        std::fs::write(d.path().join("Default/Preferences"), r#"{"browser":{"other":1},"x":2}"#).unwrap();
        write_theme(d.path(), 0x336699).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("Default/Preferences")).unwrap()).unwrap();
        assert_eq!(v["browser"]["other"], 1);
        assert_eq!(v["x"], 2);
        assert_eq!(v["browser"]["theme"]["user_color2"], 0xFF336699u32 as i32);
        assert_eq!(v["browser"]["theme"]["user_color"], v["browser"]["theme"]["user_color2"]);
    }

    /// Not a test: the fake browser process re-runs this binary on this function (see [`crate::fake`]).
    #[test]
    fn fake_browser_process() {
        crate::fake::serve_if_requested();
    }

    #[cfg(unix)]
    fn launch_fake(mode: crate::fake::Mode, tabs: usize) -> (tempfile::TempDir, crate::fake::FakeBrowser, Chromium) {
        let d = tempfile::tempdir().unwrap();
        let fake = crate::fake::FakeBrowser::install(d.path(), "launch::tests::fake_browser_process", mode, tabs).unwrap();
        let exe = Browser {
            flavor: crate::Flavor::Chromium,
            path: fake.exe.clone(),
        };
        let b = Chromium::launch(&exe, &LaunchOptions::default()).expect("the fake starts");
        (d, fake, b)
    }

    #[cfg(unix)]
    #[test]
    fn the_launch_command_line_has_no_offscreen_or_headless_argument() {
        let (_d, fake, b) = launch_fake(crate::fake::Mode::Obey, 1);
        let args = fake.args();
        assert!(args.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert!(args.iter().any(|a| a == "--enable-automation"));
        assert!(!args
            .iter()
            .any(|a| a.contains("window-position") || a.contains("headless") || a.contains("-32000")));
        drop(b);
    }

    #[cfg(unix)]
    #[test]
    fn close_gracefully_lets_the_browser_exit_and_removes_the_profile() {
        let (_d, _fake, mut b) = launch_fake(crate::fake::Mode::Obey, 1);
        let (dir, pid) = (b.profile_dir().to_path_buf(), b.pid().unwrap());
        assert!(dir.exists());
        assert!(b.close_gracefully(Duration::from_secs(5)), "it exited by itself");
        assert!(!dir.exists(), "the throwaway profile is gone");
        assert!(!crate::process::process_exists(pid) || !b.is_running());
    }

    #[cfg(unix)]
    #[test]
    fn a_browser_that_ignores_close_is_killed_after_the_timeout() {
        let (_d, _fake, mut b) = launch_fake(crate::fake::Mode::IgnoreClose, 1);
        let dir = b.profile_dir().to_path_buf();
        let start = Instant::now();
        assert!(!b.close_gracefully(Duration::from_millis(400)), "it had to be ended by force");
        assert!(start.elapsed() < Duration::from_secs(5), "bounded");
        assert!(!b.is_running());
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn page_count_sees_the_pages_the_browser_has() {
        let (_d, _fake, b) = launch_fake(crate::fake::Mode::Obey, 3);
        assert_eq!(b.page_count().unwrap(), 3);
    }

    #[cfg(unix)]
    fn wait_gone(b: &mut Chromium, p: &Page) -> Gone {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(g) = b.gone(p) {
                return g;
            }
            assert!(Instant::now() < end, "the end of the browser was not seen");
            std::thread::yield_now();
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_user_closing_the_window_ends_the_page_while_the_process_runs() {
        let (_d, fake, mut b) = launch_fake(crate::fake::Mode::Obey, 1);
        let p = b.new_page().unwrap();
        assert_eq!(b.gone(&p), None);
        assert!(p.exists().unwrap());
        fake.act(crate::fake::UserAction::CloseWindow).unwrap();
        assert_eq!(wait_gone(&mut b, &p), Gone::PageClosed);
        assert!(b.is_running(), "the browser itself still runs (macOS keeps it without a window)");
        assert!(!p.exists().unwrap());
        assert!(matches!(p.evaluate("1"), Err(Error::Closed)), "every call says the page was closed");
        assert!(matches!(p.info(), Err(Error::Closed)));
        assert!(matches!(
            p.navigate("http://127.0.0.1:9/", Duration::from_secs(5)),
            Err(Error::Closed)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_load_in_progress_ends_at_once_when_the_window_is_closed() {
        let (_d, fake, b) = launch_fake(crate::fake::Mode::Obey, 1);
        let p = b.new_page().unwrap();
        fake.act(crate::fake::UserAction::StallLoads).unwrap();
        let started = Instant::now();
        let load = std::thread::spawn(move || p.navigate("http://127.0.0.1:9/", Duration::from_secs(20)));
        fake.act(crate::fake::UserAction::CloseWindow).unwrap();
        let r = load.join().unwrap();
        assert!(matches!(r, Err(Error::Closed)), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "no wait for the load timeout");
    }

    #[cfg(unix)]
    #[test]
    fn quitting_the_browser_is_seen_as_exited() {
        let (_d, fake, mut b) = launch_fake(crate::fake::Mode::Obey, 1);
        let p = b.new_page().unwrap();
        fake.act(crate::fake::UserAction::Quit).unwrap();
        assert!(matches!(wait_gone(&mut b, &p), Gone::Exited | Gone::Disconnected));
        let end = Instant::now() + Duration::from_secs(5);
        while b.gone(&p) != Some(Gone::Exited) {
            assert!(Instant::now() < end, "the process end is seen");
            std::thread::yield_now();
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_lost_connection_is_seen_while_the_process_runs() {
        let (_d, fake, mut b) = launch_fake(crate::fake::Mode::Obey, 1);
        let p = b.new_page().unwrap();
        fake.act(crate::fake::UserAction::DropConnection).unwrap();
        assert_eq!(wait_gone(&mut b, &p), Gone::Disconnected);
        assert!(matches!(p.evaluate("1"), Err(Error::Closed)));
        b.kill();
    }

    #[test]
    fn missing_executable_is_a_launch_error() {
        let exe = Browser {
            flavor: crate::Flavor::Chromium,
            path: "/nonexistent/chrome".into(),
        };
        let e = Chromium::launch(&exe, &LaunchOptions::default()).err().unwrap();
        assert!(matches!(e, Error::Launch(_)));
    }
}
