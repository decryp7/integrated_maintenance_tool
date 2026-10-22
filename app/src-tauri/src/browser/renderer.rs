//! The renderer (ADR 0043): a real Chromium-based browser driven over the DevTools protocol, one per Debrief window and
//! started on demand. Research and the chat's browser modes read and drive its page through the `debrief_ai::browser`
//! ports, with the same scripts as before (evaluated over the protocol, replies checked by a one-shot gate).
//!
//! Nothing here depends on Tauri, so it is tested (and live-tested) without an app. Indicators that the AI drives the
//! page are all outside the page: Chrome's automation banner, the frame colour, the status chip (with Stop) and the
//! research log. Nothing is ever injected into a page.

use chromium_driver::cue::{window_title, Activity};
use chromium_driver::launch::{LaunchOptions, Profile, Visibility};
use chromium_driver::{Chromium, Error as DriverError, Page};
use debrief_ai::browser::policy::ScrollDirection;
use debrief_ai::browser::{
    capture_from, scripts, snapshot_from, BrowserError, ChatBrowser, DomSnapshot, Element, Nonce, PageCapture, PageDriver, PageReader,
    ReplyError, ReplyGate, ResultsPage, ResultsReader, WebUrl,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// The frame colour of the AI's browser window (Debrief's accent).
pub const ACCENT: u32 = 0x2F6FE0;
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_millis(400);
const ACT_LOAD_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a challenge page may wait for the user.
#[cfg(not(test))]
const CHALLENGE_WAIT: Duration = Duration::from_secs(10 * 60);
#[cfg(test)]
const CHALLENGE_WAIT: Duration = Duration::from_secs(25);

/// Where a browser may come from, in order: the user's setting, the one Debrief installed, one found on the machine.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub configured: Option<PathBuf>,
    pub installed: Option<PathBuf>,
}

impl Sources {
    pub fn resolve(&self) -> Option<chromium_driver::BrowserExe> {
        let mk = |p: &PathBuf| {
            p.is_file().then(|| chromium_driver::BrowserExe {
                flavor: chromium_driver::Flavor::Chromium,
                path: p.clone(),
            })
        };
        self.configured
            .as_ref()
            .and_then(mk)
            .or_else(|| self.installed.as_ref().and_then(mk))
            .or_else(chromium_driver::find)
    }
}

/// One window's running browser and its page.
pub struct Session {
    chromium: Mutex<Chromium>,
    page: Page,
    visible: std::sync::atomic::AtomicBool,
}

impl Session {
    #[cfg(test)]
    pub fn page(&self) -> &Page {
        &self.page
    }

    fn show(&self, on: bool) {
        use std::sync::atomic::Ordering::SeqCst;
        if self.visible.swap(on, SeqCst) != on {
            if let Err(e) = self.page.set_visible(on) {
                log::warn!("The browser window could not be moved: {e}");
            }
        }
    }

    fn running(&self) -> bool {
        crate::sync::lock(&self.chromium).is_running()
    }

    fn kill(&self) {
        crate::sync::lock(&self.chromium).kill();
    }

    #[cfg(test)]
    pub fn pid(&self) -> Option<u32> {
        crate::sync::lock(&self.chromium).pid()
    }
}

/// The renderers of the open windows.
#[derive(Default)]
pub struct Renderers {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    sources: RwLock<Sources>,
}

impl Renderers {
    #[cfg(test)]
    pub fn set_sources(&self, s: Sources) {
        *crate::sync::write(&self.sources) = s;
    }

    pub fn set_configured(&self, p: Option<PathBuf>) {
        crate::sync::write(&self.sources).configured = p;
    }

    pub fn set_installed(&self, p: Option<PathBuf>) {
        crate::sync::write(&self.sources).installed = p;
    }

    /// The window's session, started when needed. `visible` brings the window on screen (never hides one that is).
    pub fn session(&self, owner: &str, visible: bool) -> Result<Arc<Session>, BrowserError> {
        let mut map = crate::sync::lock(&self.sessions);
        if let Some(s) = map.get(owner).filter(|s| s.running()).cloned() {
            if visible {
                s.show(true);
            }
            return Ok(s);
        }
        map.remove(owner);
        let exe = crate::sync::read(&self.sources).resolve().ok_or_else(|| {
            BrowserError::Failed(
                "no Chromium-based browser was found: install Edge, Chrome or Chromium, or download one in Settings ▸ Web browser".into(),
            )
        })?;
        let opts = LaunchOptions {
            profile: Profile::Throwaway,
            visibility: if visible { Visibility::OnScreen } else { Visibility::OffScreen },
            accent: Some(ACCENT),
            ..LaunchOptions::default()
        };
        let chromium = Chromium::launch(&exe, &opts).map_err(failed)?;
        let page = chromium.new_page().map_err(failed)?;
        if !visible {
            let _ = page.block_heavy(true);
        }
        let s = Arc::new(Session {
            chromium: Mutex::new(chromium),
            page,
            visible: std::sync::atomic::AtomicBool::new(visible),
        });
        map.insert(owner.to_string(), s.clone());
        Ok(s)
    }

    pub fn current(&self, owner: &str) -> Option<Arc<Session>> {
        crate::sync::lock(&self.sessions).get(owner).filter(|s| s.running()).cloned()
    }

    /// Ends the window's browser at once; true when one was running.
    pub fn kill(&self, owner: &str) -> bool {
        let s = crate::sync::lock(&self.sessions).remove(owner);
        s.map(|s| s.kill()).is_some()
    }

    pub fn kill_all(&self) {
        let all: Vec<_> = crate::sync::lock(&self.sessions).drain().map(|(_, s)| s).collect();
        for s in all {
            s.kill();
        }
    }
}

impl Drop for Renderers {
    fn drop(&mut self) {
        self.kill_all();
    }
}

fn failed(e: DriverError) -> BrowserError {
    match e {
        DriverError::Timeout(_) => BrowserError::Timeout,
        DriverError::Closed => BrowserError::Failed("the browser was closed".into()),
        other => BrowserError::Failed(other.to_string()),
    }
}

/// What the status chip shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub activity: Activity,
    pub host: String,
    /// The title the window would carry: `"<product> AI — reading: host"`.
    pub title: String,
}

/// A window's page as the chat ports: read by everyone, driven only while `may_drive` says so.
pub struct ChromiumPage {
    pub renderers: Arc<Renderers>,
    pub owner: String,
    /// Show the window on screen (read-along, agent); research keeps it off screen until a page needs the user.
    pub on_screen: bool,
    pub may_drive: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    /// Site rules of the driving chat, checked after every navigation.
    pub rules: Option<Arc<ChatBrowser>>,
    pub product: String,
    pub notify: Arc<dyn Fn(Cue) + Send + Sync>,
}

impl ChromiumPage {
    fn session(&self) -> Result<Arc<Session>, BrowserError> {
        self.renderers.session(&self.owner, self.on_screen)
    }

    fn cue(&self, activity: Activity, url: &str) {
        let host = WebUrl::parse(url).map(|u| u.site().as_str().to_string()).unwrap_or_default();
        (self.notify)(Cue {
            activity,
            title: window_title(&self.product, activity, &host),
            host,
        });
    }

    fn eval(&self, script: impl FnOnce(&Nonce) -> String) -> Result<Map<String, Value>, BrowserError> {
        let s = self.session()?;
        let url = s.page.info().map(|i| i.url).unwrap_or_default();
        let origin = WebUrl::parse(&url).ok().map(|u| u.origin());
        let nonce = Nonce::new();
        let mut gate = ReplyGate::new(nonce.clone(), origin);
        let v = s.page.evaluate(&script(&nonce)).map_err(failed)?;
        let raw = v.as_str().ok_or_else(|| BrowserError::Failed("the page did not answer".into()))?;
        gate.accept(raw).map_err(|e| match e {
            ReplyError::Script(s) if s == "gone" => BrowserError::PageChanged("the element is gone".into()),
            ReplyError::Script(s) if s == "changed" => BrowserError::PageChanged("the element is a different one now".into()),
            ReplyError::Script(s) if s == "password" => BrowserError::Failed("refused: that is a password field".into()),
            other => other.into(),
        })
    }

    fn driving(&self) -> Result<(), BrowserError> {
        match &self.may_drive {
            Some(f) if f() => Ok(()),
            _ => Err(BrowserError::Failed("another chat is driving the browser now".into())),
        }
    }

    /// A challenge page (consent, CAPTCHA) comes to the front for the user; once it is gone the page goes back to
    /// where it was and the work continues.
    fn hand_off_challenges(&self, s: &Session) -> Result<(), BrowserError> {
        if s.page.challenge().ok().flatten().is_none() {
            return Ok(());
        }
        let was_visible = s.visible.load(std::sync::atomic::Ordering::SeqCst);
        s.show(true);
        let url = s.page.info().map(|i| i.url).unwrap_or_default();
        self.cue(Activity::WaitingForYou, &url);
        let end = Instant::now() + CHALLENGE_WAIT;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if !s.running() {
                return Err(BrowserError::Failed("the browser was closed".into()));
            }
            match s.page.challenge() {
                Ok(None) => break,
                Err(e) => return Err(failed(e)),
                Ok(Some(_)) if Instant::now() > end => {
                    return Err(BrowserError::Failed(
                        "the page still needs you (a check or consent); open the page yourself or try again".into(),
                    ))
                }
                Ok(Some(_)) => {}
            }
        }
        if !was_visible && !self.on_screen {
            s.show(false);
        }
        Ok(())
    }

    /// After an action or navigation: wait for the load it started, hand off challenges, enforce the chat's site rules.
    fn settle(&self, s: &Session, activity: Activity) -> Result<(), BrowserError> {
        std::thread::sleep(SETTLE);
        let end = Instant::now() + ACT_LOAD_TIMEOUT;
        while !s.page.load_finished().unwrap_or(true) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        self.hand_off_challenges(s)?;
        let url = s.page.info().map(|i| i.url).unwrap_or_default();
        if let (Some(rules), Ok(web)) = (&self.rules, WebUrl::parse(&url)) {
            if !rules.allows_navigation(&web) {
                let _ = s.page.back(Duration::from_secs(5));
                return Err(BrowserError::Failed(format!(
                    "blocked: this chat may not visit {}",
                    web.site().as_str()
                )));
            }
        }
        self.cue(activity, &url);
        Ok(())
    }

    fn act(&self, script: impl FnOnce(&Nonce) -> String) -> Result<(), BrowserError> {
        self.driving()?;
        self.eval(script)?;
        let s = self.session()?;
        self.settle(&s, Activity::Reading)
    }
}

impl PageReader for ChromiumPage {
    fn capture(&self) -> Result<PageCapture, BrowserError> {
        let m = self.eval(scripts::capture)?;
        let url = m.get("url").and_then(Value::as_str).unwrap_or("").to_string();
        self.cue(Activity::Reading, &url);
        Ok(capture_from(&m))
    }

    fn snapshot(&self) -> Result<DomSnapshot, BrowserError> {
        self.eval(scripts::snapshot).map(|m| snapshot_from(&m))
    }
}

impl ResultsReader for ChromiumPage {
    fn results_page(&self, region: &[&str]) -> Result<ResultsPage, BrowserError> {
        self.eval(|n| scripts::result_links(n, region))
            .map(|m| debrief_ai::browser::results_page_from(&m))
    }
}

impl PageDriver for ChromiumPage {
    fn navigate(&self, url: &WebUrl) -> Result<(), BrowserError> {
        self.driving()?;
        let s = self.session()?;
        self.cue(Activity::Searching, url.as_str());
        match s.page.navigate(url.as_str(), LOAD_TIMEOUT) {
            Ok(()) | Err(DriverError::Timeout(_)) => {}
            Err(e) => return Err(failed(e)),
        }
        self.settle(&s, Activity::Reading)
    }

    fn click(&self, element: &Element) -> Result<(), BrowserError> {
        self.act(|n| scripts::click(n, element))
    }

    fn type_text(&self, element: &Element, text: &str, submit: bool) -> Result<(), BrowserError> {
        self.act(|n| scripts::type_text(n, element, text, submit))
    }

    fn scroll(&self, direction: ScrollDirection) -> Result<(), BrowserError> {
        self.act(|n| scripts::scroll(n, direction))
    }

    fn back(&self) -> Result<(), BrowserError> {
        self.driving()?;
        let s = self.session()?;
        s.page.back(ACT_LOAD_TIMEOUT).map_err(failed)?;
        self.settle(&s, Activity::Reading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_browser_gives_a_message_that_says_what_to_do() {
        let r = Renderers::default();
        r.set_sources(Sources {
            configured: Some("/nonexistent/chrome".into()),
            installed: None,
        });
        if chromium_driver::find().is_none() {
            let e = r.session("main", false).err().unwrap();
            assert!(e.to_string().contains("Settings"));
        }
        assert!(!r.kill("main"));
    }

    #[test]
    fn configured_path_wins_over_installed() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let s = Sources {
            configured: Some(a.clone()),
            installed: Some(b.clone()),
        };
        assert_eq!(s.resolve().unwrap().path, a);
        let s = Sources {
            configured: Some(d.path().join("missing")),
            installed: Some(b.clone()),
        };
        assert_eq!(s.resolve().unwrap().path, b);
    }

    // ---- a real browser, when the machine has one (skipped otherwise) ----

    fn serve() -> u16 {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut b = [0u8; 2048];
                let n = s.read(&mut b).unwrap_or(0);
                let first = String::from_utf8_lossy(&b[..n]).lines().next().unwrap_or("").to_string();
                let body = if first.contains("/check") {
                    // A human check that goes away after three seconds, as if the user had solved it.
                    "<title>Just a moment</title><body><p id=m>Verify you are human to continue</p><script>setTimeout(()=>{document.title='Done';m.textContent='Welcome. This is the real page.'},3000)</script></body>"
                } else {
                    "<title>Results</title><body><div id=r></div><script>setTimeout(()=>{for(let i=1;i<=3;i++){const a=document.createElement('a');a.href='https://example.org/'+i;a.textContent='Result '+i;r.appendChild(a)}},200)</script></body>"
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    type Live = (Arc<Renderers>, ChromiumPage, Arc<Mutex<Vec<Cue>>>);

    fn live_page(on_screen: bool) -> Option<Live> {
        chromium_driver::find()?;
        let renderers = Arc::new(Renderers::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let page = ChromiumPage {
            renderers: renderers.clone(),
            owner: "main".into(),
            on_screen,
            may_drive: Some(Box::new(|| true)),
            rules: None,
            product: "Debrief".into(),
            notify: Arc::new(move |c| s2.lock().unwrap().push(c)),
        };
        Some((renderers, page, seen))
    }

    #[test]
    fn reads_a_scripted_page_through_the_ports_with_nothing_of_ours_in_it() {
        let Some((renderers, page, seen)) = live_page(false) else { return };
        let port = serve();
        let url = WebUrl::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        page.navigate(&url).expect("navigate");
        let mut links = 0;
        for _ in 0..30 {
            links = page.results_page(&[]).map(|p| p.links.len()).unwrap_or(0);
            if links == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(links, 3, "the script-rendered results are read");
        let cap = page.capture().unwrap();
        assert!(cap.text.contains("Result 1"));
        // Nothing of ours in the page: no node, attribute or text added by Debrief.
        let s = renderers.current("main").unwrap();
        let html = s.page().evaluate("document.documentElement.outerHTML").unwrap();
        let html = html.as_str().unwrap();
        assert!(
            !html.to_lowercase().contains("debrief") && !html.contains("AI is browsing"),
            "{html}"
        );
        assert_eq!(
            s.page().evaluate("document.querySelectorAll('script').length").unwrap(),
            serde_json::json!(1)
        );
        // The cue went to the status chip, with the product name in the window title.
        let cues = seen.lock().unwrap().clone();
        assert!(cues
            .iter()
            .any(|c| c.title == "Debrief AI — reading: 127.0.0.1" || c.title.starts_with("Debrief AI — searching")));
        // Stop: the process ends at once and later calls fail with a clear message.
        let pid = s.pid().unwrap();
        assert!(renderers.kill("main"));
        #[cfg(unix)]
        {
            std::thread::sleep(Duration::from_millis(300));
            assert!(!chromium_driver::process::process_exists(pid), "browser process leaked");
        }
        let _ = pid;
        assert!(renderers.current("main").is_none());
    }

    #[test]
    fn a_challenge_page_comes_to_the_front_and_the_work_continues_when_it_is_solved() {
        let Some((renderers, page, seen)) = live_page(false) else { return };
        let port = serve();
        let url = WebUrl::parse(&format!("http://127.0.0.1:{port}/check")).unwrap();
        page.navigate(&url).expect("navigates once the check is gone");
        let cues = seen.lock().unwrap().clone();
        assert!(cues.iter().any(|c| c.activity == Activity::WaitingForYou), "the user was asked");
        assert!(cues.last().unwrap().activity == Activity::Reading, "and it went on afterwards");
        assert!(page.capture().unwrap().text.contains("real page"));
        renderers.kill_all();
    }
}
