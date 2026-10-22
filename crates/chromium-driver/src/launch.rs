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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Headful but moved off screen.
    OffScreen,
    OnScreen,
}

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
    pub visibility: Visibility,
    /// Window frame colour as `0xRRGGBB` (best effort; see [`theme_preferences`]).
    pub accent: Option<u32>,
    /// The browser is ended after this long, whatever it does.
    pub max_lifetime: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        LaunchOptions {
            profile: Profile::Throwaway,
            visibility: Visibility::OffScreen,
            accent: None,
            max_lifetime: Duration::from_secs(30 * 60),
        }
    }
}

/// The command-line arguments. `--enable-automation` keeps Chrome's "controlled by automated test software"
/// banner and `navigator.webdriver = true`; nothing hides either.
pub fn args(data_dir: &Path, visibility: Visibility) -> Vec<String> {
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
    if visibility == Visibility::OffScreen {
        a.push("--window-position=-32000,-32000".into());
    }
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
    let mut root: Value = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
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

fn unique_dir() -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("chromium-driver-{}-{n}", std::process::id()))
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
        cmd.args(args(&dir, opts.visibility))
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

    pub fn profile_dir(&self) -> &Path {
        &self.dir
    }

    /// Opens a page (tab) to drive.
    pub fn new_page(&self) -> Result<Page> {
        Page::attach(self.conn.clone())
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
    fn keeps_the_automation_banner_and_hides_nothing() {
        let a = args(Path::new("/tmp/p"), Visibility::OffScreen);
        assert!(a.contains(&"--enable-automation".to_string()));
        assert!(a.iter().all(|x| !x.contains("AutomationControlled") && !x.contains("user-agent") && !x.contains("headless")));
        assert!(a.contains(&"--window-position=-32000,-32000".to_string()));
        assert!(a.contains(&"--user-data-dir=/tmp/p".to_string()));
    }

    #[test]
    fn on_screen_has_no_offscreen_position() {
        assert!(!args(Path::new("/p"), Visibility::OnScreen).iter().any(|x| x.starts_with("--window-position")));
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

    #[test]
    fn missing_executable_is_a_launch_error() {
        let exe = Browser { flavor: crate::Flavor::Chromium, path: "/nonexistent/chrome".into() };
        let e = Chromium::launch(&exe, &LaunchOptions::default()).err().unwrap();
        assert!(matches!(e, Error::Launch(_)));
    }
}
