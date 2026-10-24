//! The renderer (ADR 0043): a real Chromium-based browser driven over the DevTools protocol, one per Debrief window and
//! started on demand. Research and the chat's browser modes read and drive its page through the `debrief_ai::browser`
//! ports, with the same scripts as before (evaluated over the protocol, replies checked by a one-shot gate).
//!
//! Nothing here depends on Tauri, so it is tested (and live-tested) without an app. Indicators that the AI drives the
//! page are all outside the page: Chrome's automation banner, the frame colour, the status chip (with Stop) and the
//! research log. Nothing is ever injected into a page.
//!
//! The browsers themselves and their life cycle are [`super::sessions`] (ADR 0057).

use super::sessions::{Lease, Renderers, Session};
use chromium_driver::cue::{window_title, Activity};
use chromium_driver::Error as DriverError;
use debrief_ai::browser::policy::ScrollDirection;
use debrief_ai::browser::{
    capture_from, scripts, snapshot_from, BrowserError, ChatBrowser, DomSnapshot, Element, Nonce, PageCapture, PageDriver, PageReader,
    ReplyError, ReplyGate, ResultsPage, ResultsReader, WebUrl,
};
use serde_json::{Map, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
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

pub(super) fn failed(e: DriverError) -> BrowserError {
    match e {
        DriverError::Timeout(_) => BrowserError::Timeout,
        DriverError::Closed => BrowserError::Closed,
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
    pub may_drive: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    /// Site rules of the driving chat, checked after every navigation.
    pub rules: Option<Arc<ChatBrowser>>,
    pub product: String,
    pub notify: Arc<dyn Fn(Cue) + Send + Sync>,
    /// The run this page belongs to (a research): once its browser is gone, the page fails instead of starting another.
    pub lease: Lease,
}

impl ChromiumPage {
    /// The same page under a run's lease (see [`Lease`]).
    pub fn with_lease(self, lease: Lease) -> Self {
        ChromiumPage { lease, ..self }
    }

    fn session(&self) -> Result<Arc<Session>, BrowserError> {
        self.renderers.session_in(&self.owner, &self.lease)
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

    /// A challenge page (consent, CAPTCHA) waits for the user in the browser window, which is already on screen; once
    /// it is gone the work continues. The user took part, so the window is not closed when the work ends.
    fn hand_off_challenges(&self, s: &Session) -> Result<(), BrowserError> {
        if s.page.challenge().ok().flatten().is_none() {
            return Ok(());
        }
        s.handed_off.store(true, Ordering::SeqCst);
        let url = s.page.info().map(|i| i.url).unwrap_or_default();
        self.cue(Activity::WaitingForYou, &url);
        let end = Instant::now() + CHALLENGE_WAIT;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if s.gone().is_some() {
                return Err(BrowserError::Closed);
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
        s.remember_url();
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
    use super::super::lifecycle::{GoneReason, Lifecycle, Phase};
    use super::super::sessions::{Finish, Sources, TookPart, QUIT_GRACE};
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn no_browser_gives_a_message_that_says_what_to_do() {
        let r = Renderers::default();
        r.set_sources(Sources {
            configured: Some("/nonexistent/chrome".into()),
            installed: None,
        });
        if chromium_driver::find_for_tests().is_none() {
            let e = r.session_in("main", &Lease::default()).err().unwrap();
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

    // ---- a fake browser (never a real one): the whole open/close life cycle ----

    /// Not a test: the fake browser process re-runs this binary on this function.
    #[test]
    fn fake_browser_process() {
        chromium_driver::fake::serve_if_requested();
    }

    #[cfg(unix)]
    mod fake {
        use super::*;
        use chromium_driver::fake::{FakeBrowser, Mode, UserAction};

        struct Rig {
            _dir: tempfile::TempDir,
            fake: FakeBrowser,
            renderers: Arc<Renderers>,
            page: ChromiumPage,
        }

        fn rig(mode: Mode, tabs: usize) -> Rig {
            let dir = tempfile::tempdir().unwrap();
            let fake = FakeBrowser::install(dir.path(), "browser::renderer::tests::fake_browser_process", mode, tabs).unwrap();
            let renderers = Arc::new(Renderers::default());
            renderers.set_sources(Sources {
                configured: Some(fake.exe.clone()),
                installed: None,
            });
            let page = ChromiumPage {
                renderers: renderers.clone(),
                owner: "main".into(),
                may_drive: Some(Box::new(|| true)),
                rules: None,
                product: "Debrief".into(),
                notify: Arc::new(|_| {}),
                lease: Lease::default(),
            };
            Rig {
                _dir: dir,
                fake,
                renderers,
                page,
            }
        }

        fn research_step(r: &Rig) -> (std::path::PathBuf, u32) {
            r.page
                .navigate(&WebUrl::parse("http://127.0.0.1:9/results").unwrap())
                .expect("navigates");
            let s = r
                .renderers
                .current("main")
                .unwrap_or_else(|| panic!("running: {:?}", r.renderers.lifecycle("main")));
            let dir = crate::sync::lock(&s.chromium).profile_dir().to_path_buf();
            (dir, s.pid().unwrap())
        }

        #[test]
        fn service_health_follows_the_browser_life_cycle() {
            use crate::health::{hub::Health, model::RepairAction, model::ServiceState};
            use tauri::Manager;
            let app = crate::test_support::test_app(None);
            let r = rig(Mode::Obey, 1);
            crate::browser::watch_health(app.handle(), &r.renderers);
            let health = app.state::<Arc<Health>>().inner().clone();
            let browser = || health.snapshot().services.into_iter().find(|s| s.service == "browser").unwrap();
            assert_eq!(browser().state, ServiceState::Ok, "nothing started: fine");
            research_step(&r);
            assert_eq!(browser().state, ServiceState::Ok, "ready");
            // A start that fails (the chosen browser is not one): down, fixed in Settings ▸ Web browser.
            r.renderers.kill("main");
            let not_a_browser = r._dir.path().join("not-a-browser.txt");
            std::fs::write(&not_a_browser, "text").unwrap();
            r.renderers.set_sources(Sources {
                configured: Some(not_a_browser),
                installed: None,
            });
            assert!(r.renderers.session_in("main", &Lease::default()).is_err());
            let b = browser();
            assert_eq!((b.state, b.repair), (ServiceState::Down, Some(RepairAction::OpenSettings)));
            assert_eq!(b.cause.and_then(|c| c.settings).as_deref(), Some(crate::browser::SECTION));
            // The user fixes it there; the next start works and the outage is over.
            r.renderers.set_sources(Sources {
                configured: Some(r.fake.exe.clone()),
                installed: None,
            });
            research_step(&r);
            assert_eq!(browser().state, ServiceState::Ok);
            r.renderers.kill_all();
        }

        /// A browser that ended by itself is cleaned up by the watcher thread: its profile goes shortly after the process.
        fn removed(dir: &std::path::Path) -> bool {
            (0..100).any(|_| {
                std::thread::sleep(Duration::from_millis(50));
                !dir.exists()
            })
        }

        fn gone(pid: u32) -> bool {
            (0..40).any(|_| {
                std::thread::sleep(Duration::from_millis(50));
                !chromium_driver::process::process_exists(pid)
            })
        }

        #[test]
        fn the_browser_opens_on_screen_with_no_offscreen_or_headless_arguments() {
            let r = rig(Mode::Obey, 1);
            research_step(&r);
            let args = r.fake.args();
            assert!(args.iter().any(|a| a.starts_with("--user-data-dir=")), "{args:?}");
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("window-position") || a.contains("headless") || a.contains("-32000")),
                "{args:?}"
            );
            r.renderers.kill_all();
        }

        #[test]
        fn the_browser_opens_where_the_owner_window_s_display_says() {
            let r = rig(Mode::Obey, 1);
            let asked = Arc::new(Mutex::new(Vec::new()));
            let a2 = asked.clone();
            r.renderers.set_placer(Arc::new(move |owner: &str| {
                a2.lock().unwrap().push(owner.to_string());
                Some(chromium_driver::WindowBounds {
                    x: -1650,
                    y: 100,
                    width: 1100,
                    height: 800,
                })
            }));
            research_step(&r);
            assert_eq!(*asked.lock().unwrap(), ["main"], "placed for the window that started it");
            let args = r.fake.args();
            assert!(args.contains(&"--window-position=-1650,100".to_string()), "{args:?}");
            assert!(args.contains(&"--window-size=1100,800".to_string()), "{args:?}");
            assert!(!args.iter().any(|a| a.contains("headless") || a.contains("-32000")), "{args:?}");
            r.renderers.kill_all();
        }

        #[test]
        fn finished_research_closes_the_browser_and_removes_its_profile() {
            let r = rig(Mode::Obey, 1);
            let (dir, pid) = research_step(&r);
            assert!(dir.exists());
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(gone(pid), "the browser process ended");
            assert!(!dir.exists(), "the throwaway profile is gone");
            assert!(r.renderers.current("main").is_none());
            assert_eq!(r.renderers.finish("main"), Finish::NotRunning);
        }

        /// Regression (1.1.0): a research nobody touched left its browser open. A real browser opens with its start
        /// page, and the driver opened a second page beside it, so "more than one page" always said the user took part.
        #[test]
        fn a_research_nobody_touched_closes_its_browser_when_it_ends() {
            use crate::health::{hub::Health, model::ServiceState};
            use tauri::Manager;
            let app = crate::test_support::test_app(None);
            let r = rig(Mode::Obey, 1);
            crate::browser::watch_health(app.handle(), &r.renderers);
            let health = app.state::<Arc<Health>>().inner().clone();
            let seen = records(&r);
            // One research: its reader and driver share a run's lease, as `research::run_research` does.
            let lease = Lease::run();
            let (reader, driver) = (page_with(&r, lease.clone()), page_with(&r, lease));
            driver.navigate(&WebUrl::parse("http://127.0.0.1:9/results").unwrap()).unwrap();
            driver.navigate(&WebUrl::parse("http://127.0.0.1:9/result-1").unwrap()).unwrap();
            drop(reader);
            let s = r.renderers.current("main").unwrap();
            let dir = crate::sync::lock(&s.chromium).profile_dir().to_path_buf();
            let pid = s.pid().unwrap();
            let pages = crate::sync::lock(&s.chromium).page_count().unwrap();
            assert_eq!(pages, 1, "Debrief drives the start page, not a second one");
            drop(s);
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(
                r.fake.calls().iter().any(|c| c == "Browser.close"),
                "closed the way a user would first"
            );
            assert!(gone(pid), "the browser process ended");
            assert!(!dir.exists(), "the throwaway profile is gone");
            let phases: Vec<Phase> = seen.lock().unwrap().iter().map(|l| l.phase).collect();
            assert_eq!(
                phases,
                [
                    Phase::Launching,
                    Phase::Ready,
                    Phase::Closing,
                    Phase::Gone { reason: GoneReason::Ended }
                ],
                "the window hears it closing and gone (ended)"
            );
            assert!(!seen.lock().unwrap().iter().any(|l| l.kept), "never marked kept");
            let browser = health.snapshot().services.into_iter().find(|s| s.service == "browser").unwrap();
            assert_eq!(browser.state, ServiceState::Ok, "ended by Debrief is not an outage");
        }

        #[test]
        fn a_browser_that_ignores_close_is_killed_after_the_timeout() {
            let r = rig(Mode::IgnoreClose, 1);
            let (dir, pid) = research_step(&r);
            let start = Instant::now();
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(start.elapsed() < Duration::from_secs(10), "bounded");
            assert!(gone(pid), "killed");
            assert!(!dir.exists());
        }

        #[test]
        fn a_page_that_needed_the_user_keeps_the_browser_open_until_they_close_it() {
            let r = rig(Mode::Obey, 1);
            let (dir, pid) = research_step(&r);
            r.renderers.current("main").unwrap().handed_off.store(true, Ordering::SeqCst);
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::PageNeededThem));
            assert!(chromium_driver::process::process_exists(pid) && dir.exists());
            assert!(r.renderers.close("main"), "the user's Close");
            assert!(gone(pid));
            assert!(!dir.exists());
            assert!(!r.renderers.close("main"), "closing twice is harmless");
        }

        #[test]
        fn the_user_navigating_the_page_keeps_the_browser_open() {
            let r = rig(Mode::Obey, 1);
            let (dir, _) = research_step(&r);
            std::fs::write(&r.fake.url_file, "https://example.org/the-user-went-here").unwrap();
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::Navigated));
            assert!(dir.exists());
            r.renderers.kill_all();
        }

        #[test]
        fn a_page_the_user_opened_keeps_the_browser_open() {
            let r = rig(Mode::Obey, 2);
            research_step(&r);
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::OpenedPages(2)));
            r.renderers.kill_all();
        }

        #[test]
        fn the_decision_is_logged_with_its_reason_and_no_address() {
            let cases = [
                (Finish::Closed, "closed (nobody used it)"),
                (Finish::NotRunning, "no browser was running"),
                (
                    Finish::KeptOpen(TookPart::PageNeededThem),
                    "left open because a page needed the user",
                ),
                (
                    Finish::KeptOpen(TookPart::Navigated),
                    "left open because the user navigated the page",
                ),
                (
                    Finish::KeptOpen(TookPart::OpenedPages(3)),
                    "left open because the user opened another page (3 pages)",
                ),
            ];
            for (f, says) in cases {
                let line = f.describe("main");
                assert!(line.starts_with("Research in main ended: ") && line.contains(says), "{line}");
                assert_eq!(f.kept(), matches!(f, Finish::KeptOpen(_)));
            }
        }

        #[test]
        fn the_next_research_in_a_kept_browser_starts_afresh_and_closes_it_when_nobody_used_it() {
            let r = rig(Mode::Obey, 1);
            let seen = records(&r);
            let (dir, pid) = research_step(&r);
            r.renderers.current("main").unwrap().handed_off.store(true, Ordering::SeqCst);
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::PageNeededThem));
            assert!(seen.lock().unwrap().last().unwrap().kept);
            // A new research takes the kept browser: the earlier part no longer counts, and the offer to close goes.
            let run = page_with(&r, Lease::run());
            run.navigate(&WebUrl::parse("http://127.0.0.1:9/again").unwrap()).unwrap();
            assert_eq!(r.renderers.current("main").unwrap().pid(), Some(pid), "the same browser");
            let last = *seen.lock().unwrap().last().unwrap();
            assert_eq!((last.phase, last.kept), (Phase::Ready, false), "the window stops offering Close");
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(gone(pid) && !dir.exists());
        }

        fn data_dir_arg(r: &Rig) -> String {
            r.fake
                .args()
                .into_iter()
                .find_map(|a| a.strip_prefix("--user-data-dir=").map(str::to_string))
                .unwrap_or_default()
        }

        #[test]
        fn a_kept_research_profile_is_used_by_one_research_after_another() {
            let r = rig(Mode::Obey, 1);
            let kept = r._dir.path().join("research-profile");
            r.renderers
                .set_kept_profile(Some(kept.clone()), true, crate::browser::profile::DEFAULT_MAX_BYTES);
            let (dir, pid) = research_step(&r);
            assert_eq!(dir, kept);
            assert_eq!(data_dir_arg(&r), kept.display().to_string());
            assert!(
                r.fake.args().iter().any(|a| a.starts_with("--disk-cache-size=")),
                "its cache is capped"
            );
            let s = r.renderers.current("main").unwrap();
            assert!(s.kept_profile() && s.profile_note().is_none());
            drop(s);
            assert!(r.renderers.kept_profile_status().in_use);
            // A consent the user gave is in the profile.
            std::fs::write(kept.join("Cookies"), "consent").unwrap();
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(gone(pid));
            assert!(kept.join("Cookies").is_file(), "the kept profile stays after the browser closed");
            assert!(!r.renderers.kept_profile_status().in_use, "and is free again");
            let (dir2, pid2) = research_step(&r);
            assert_ne!(pid, pid2, "a new browser");
            assert_eq!(dir2, kept, "with the same profile");
            r.renderers.kill_all();
            assert!(kept.join("Cookies").is_file());
        }

        #[test]
        fn settings_hear_every_change_of_the_kept_profile() {
            let r = rig(Mode::Obey, 1);
            let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let h2 = heard.clone();
            r.renderers.listen_profile(Arc::new(move || {
                h2.fetch_add(1, Ordering::SeqCst);
            }));
            let count = || heard.load(Ordering::SeqCst);
            let kept = r._dir.path().join("research-profile");
            r.renderers
                .set_kept_profile(Some(kept), true, crate::browser::profile::DEFAULT_MAX_BYTES);
            assert_eq!(count(), 1, "the setting");
            research_step(&r);
            assert_eq!(count(), 2, "taken by a browser");
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert_eq!(count(), 3, "released when it closed");
            r.renderers.clear_kept_profile().unwrap();
            assert_eq!(count(), 4, "cleared");
        }

        #[test]
        fn without_the_setting_every_browser_gets_a_throwaway_profile() {
            let r = rig(Mode::Obey, 1);
            let (dir, _) = research_step(&r);
            assert!(!r.renderers.current("main").unwrap().kept_profile());
            assert!(
                dir.file_name().unwrap().to_string_lossy().starts_with("chromium-driver-"),
                "{dir:?}"
            );
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            assert!(!dir.exists(), "removed");
            assert!(!r.renderers.kept_profile_status().keep);
        }

        #[test]
        fn the_kept_profile_cannot_be_cleared_while_a_browser_uses_it() {
            let r = rig(Mode::Obey, 1);
            let kept = r._dir.path().join("research-profile");
            r.renderers
                .set_kept_profile(Some(kept.clone()), true, crate::browser::profile::DEFAULT_MAX_BYTES);
            research_step(&r);
            let e = r.renderers.clear_kept_profile().unwrap_err();
            assert!(e.contains("in use") && e.contains("Close its window"), "{e}");
            assert!(kept.is_dir(), "nothing removed");
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            r.renderers.clear_kept_profile().unwrap();
            assert!(!kept.exists());
            assert_eq!(r.renderers.kept_profile_status().size_bytes, 0);
        }

        #[test]
        fn a_damaged_kept_profile_falls_back_to_a_throwaway_one_and_says_so_once() {
            let r = rig(Mode::Obey, 1);
            let kept = r._dir.path().join("research-profile");
            std::fs::create_dir_all(kept.join("Default")).unwrap();
            std::fs::write(kept.join("Default/Preferences"), "{damaged").unwrap();
            r.renderers
                .set_kept_profile(Some(kept.clone()), true, crate::browser::profile::DEFAULT_MAX_BYTES);
            research_step(&r);
            let s = r.renderers.current("main").unwrap();
            assert!(!s.kept_profile(), "a throwaway one");
            let note = s.profile_note().expect("the user is told").to_string();
            assert!(
                note.contains("damaged") && note.contains("Clear it in Settings ▸ Web browser"),
                "{note}"
            );
            drop(s);
            assert_eq!(
                r.renderers.kept_profile_status().problem.as_deref(),
                Some(note.as_str()),
                "Settings shows it"
            );
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            research_step(&r);
            assert!(r.renderers.current("main").unwrap().profile_note().is_none(), "told once");
            assert_eq!(r.renderers.finish("main"), Finish::Closed);
            // The repair: Clear. The next research uses a fresh kept profile.
            r.renderers.clear_kept_profile().unwrap();
            assert!(r.renderers.kept_profile_status().problem.is_none());
            research_step(&r);
            assert!(r.renderers.current("main").unwrap().kept_profile());
            r.renderers.kill_all();
        }

        #[test]
        fn a_kept_profile_over_its_limit_or_that_will_not_start_falls_back_too() {
            let r = rig(Mode::Obey, 1);
            let kept = r._dir.path().join("research-profile");
            std::fs::create_dir_all(&kept).unwrap();
            std::fs::write(kept.join("big"), vec![0u8; 4096]).unwrap();
            r.renderers.set_kept_profile(Some(kept.clone()), true, 1000);
            research_step(&r);
            let note = r.renderers.current("main").unwrap().profile_note().map(str::to_string);
            assert!(note.as_deref().is_some_and(|n| n.contains("the limit is")), "{note:?}");
            r.renderers.kill_all();
            // A folder the browser cannot use (here: one that cannot be created) starts a throwaway browser instead.
            let blocked = r._dir.path().join("a-file");
            std::fs::write(&blocked, "x").unwrap();
            r.renderers.set_kept_profile(Some(blocked.join("research-profile")), true, 1 << 30);
            research_step(&r);
            let s = r.renderers.current("main").unwrap();
            assert!(!s.kept_profile());
            assert!(
                s.profile_note()
                    .is_some_and(|n| n.contains("did not start with the research profile")),
                "{:?}",
                s.profile_note()
            );
            drop(s);
            assert!(!r.renderers.kept_profile_status().in_use, "not held by the failed start");
            r.renderers.kill_all();
        }

        #[test]
        fn a_second_browser_while_the_kept_profile_is_in_use_gets_a_throwaway_one() {
            let r = rig(Mode::Obey, 1);
            let kept = r._dir.path().join("research-profile");
            r.renderers
                .set_kept_profile(Some(kept.clone()), true, crate::browser::profile::DEFAULT_MAX_BYTES);
            research_step(&r);
            let other = r.renderers.session_in("w-2", &Lease::default()).unwrap();
            assert!(!other.kept_profile() && other.profile_note().is_none(), "quietly a throwaway one");
            assert!(r.renderers.current("main").unwrap().kept_profile());
            drop(other);
            r.renderers.kill_all();
        }

        const SEEN: Duration = Duration::from_secs(5);

        fn page_with(r: &Rig, lease: Lease) -> ChromiumPage {
            ChromiumPage {
                renderers: r.renderers.clone(),
                owner: "main".into(),
                may_drive: Some(Box::new(|| true)),
                rules: None,
                product: "Debrief".into(),
                notify: Arc::new(|_| {}),
                lease,
            }
        }

        /// Waits until the listener heard `phase` (it hears a change right after waiters see it).
        fn heard(seen: &Mutex<Vec<Lifecycle>>, phase: Phase) -> Lifecycle {
            let end = Instant::now() + SEEN;
            loop {
                if let Some(l) = seen.lock().unwrap().last().copied().filter(|l| l.phase == phase) {
                    return l;
                }
                assert!(Instant::now() < end, "{phase:?} was not reported");
                std::thread::yield_now();
            }
        }

        /// The user does `action` to the browser; returns the phase it ends in (the watcher sees it without any use).
        fn user(r: &Rig, action: UserAction) -> Phase {
            r.fake.act(action).unwrap();
            r.renderers.wait_for("main", SEEN, |p| matches!(p, Phase::Gone { .. }))
        }

        fn records(r: &Rig) -> Arc<Mutex<Vec<Lifecycle>>> {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            r.renderers.listen(Arc::new(move |owner, l| {
                assert_eq!(owner, "main");
                s2.lock().unwrap().push(l);
            }));
            seen
        }

        const WINDOW_CLOSED: Phase = Phase::Gone {
            reason: GoneReason::WindowClosed,
        };

        #[test]
        fn research_works_again_after_the_user_closed_the_window() {
            let r = rig(Mode::Obey, 1);
            let seen = records(&r);
            let (dir, pid) = research_step(&r);
            assert_eq!(user(&r, UserAction::CloseWindow), WINDOW_CLOSED, "seen while idle");
            heard(&seen, WINDOW_CLOSED);
            assert!(gone(pid), "the browser left without a window is ended");
            assert!(removed(&dir), "its profile is removed");
            assert!(r.renderers.current("main").is_none());
            let (dir2, pid2) = research_step(&r);
            assert_ne!(pid, pid2, "a fresh browser");
            assert!(dir2.exists());
            let phases: Vec<Phase> = seen.lock().unwrap().iter().map(|l| l.phase).collect();
            assert_eq!(
                phases,
                [Phase::Launching, Phase::Ready, WINDOW_CLOSED, Phase::Launching, Phase::Ready],
                "every step is reported"
            );
            r.renderers.kill_all();
            assert!(gone(pid2));
        }

        #[test]
        fn a_kept_open_browser_the_user_closed_is_forgotten_and_research_starts_afresh() {
            let r = rig(Mode::Obey, 1);
            let seen = records(&r);
            let (dir, pid) = research_step(&r);
            r.renderers.current("main").unwrap().handed_off.store(true, Ordering::SeqCst);
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::PageNeededThem));
            assert!(r.renderers.lifecycle("main").kept);
            assert!(seen.lock().unwrap().last().unwrap().kept, "the window hears it was kept");
            assert_eq!(user(&r, UserAction::CloseWindow), WINDOW_CLOSED);
            let now = r.renderers.lifecycle("main");
            assert!(!now.kept, "the kept mark is gone with the browser");
            assert_eq!(heard(&seen, WINDOW_CLOSED), now, "and the window hears that too");
            assert!(gone(pid) && removed(&dir));
            assert!(!r.renderers.close("main"), "Close has nothing left to close");
            let (_, pid2) = research_step(&r);
            assert_ne!(pid, pid2);
            r.renderers.kill_all();
        }

        #[test]
        fn quitting_the_browser_or_losing_its_connection_ends_it_and_the_next_use_starts_a_new_one() {
            for action in [UserAction::Quit, UserAction::DropConnection] {
                let r = rig(Mode::Obey, 1);
                let (dir, pid) = research_step(&r);
                let phase = user(&r, action);
                assert!(
                    matches!(
                        phase,
                        Phase::Gone {
                            reason: GoneReason::Exited | GoneReason::Disconnected
                        }
                    ),
                    "{action:?}: {phase:?}"
                );
                assert!(gone(pid), "{action:?}: no process is left");
                assert!(removed(&dir), "{action:?}: no profile is left");
                let (_, pid2) = research_step(&r);
                assert_ne!(pid, pid2, "{action:?}");
                r.renderers.kill_all();
            }
        }

        #[test]
        fn a_closed_window_is_noticed_on_use_even_before_the_watcher() {
            let r = rig(Mode::Obey, 1);
            let (_, pid) = research_step(&r);
            let s = r.renderers.current("main").unwrap();
            r.fake.act(UserAction::CloseWindow).unwrap();
            // The page answers no more; the use that finds out moves the browser to gone and starts a new one.
            while s.gone().is_none() {
                if let Err(e) = s.page.evaluate("1") {
                    assert!(matches!(e, DriverError::Closed), "{e:?}");
                }
            }
            let (_, pid2) = research_step(&r);
            assert_ne!(pid, pid2);
            r.renderers.kill_all();
        }

        #[test]
        fn a_research_that_loses_its_browser_mid_way_fails_at_once_and_does_not_open_another() {
            let r = rig(Mode::Obey, 1);
            let lease = Lease::run();
            let (reader, driver) = (page_with(&r, lease.clone()), page_with(&r, lease));
            driver.navigate(&WebUrl::parse("http://127.0.0.1:9/results").unwrap()).unwrap();
            let pid = r.renderers.current("main").unwrap().pid().unwrap();
            // The next page never finishes loading, and the user closes the window meanwhile.
            r.fake.act(UserAction::StallLoads).unwrap();
            let started = Instant::now();
            let load = std::thread::spawn(move || driver.navigate(&WebUrl::parse("http://127.0.0.1:9/next").unwrap()));
            r.fake.act(UserAction::CloseWindow).unwrap();
            assert_eq!(load.join().unwrap(), Err(BrowserError::Closed), "the step fails");
            assert!(started.elapsed() < LOAD_TIMEOUT / 3, "without waiting for the load timeout");
            assert_eq!(reader.capture().err(), Some(BrowserError::Closed), "the rest of the run too");
            assert_eq!(
                r.renderers.wait_for("main", SEEN, |p| matches!(p, Phase::Gone { .. })),
                WINDOW_CLOSED
            );
            assert!(gone(pid));
            assert!(r.renderers.current("main").is_none(), "no other browser was opened for it");
            // Retry is a new run: it gets a new browser.
            let retry = page_with(&r, Lease::run());
            retry.navigate(&WebUrl::parse("http://127.0.0.1:9/results").unwrap()).unwrap();
            assert_ne!(r.renderers.current("main").unwrap().pid(), Some(pid));
            r.renderers.kill_all();
        }

        #[test]
        fn a_challenge_wait_ends_when_the_user_closes_the_window() {
            let r = rig(Mode::Obey, 1);
            research_step(&r);
            // A page that needs the user (a human check); they close the window instead of solving it.
            std::fs::write(&r.fake.url_file, "https://www.google.com/sorry/index?continue=x").unwrap();
            let s = r.renderers.current("main").unwrap();
            assert!(s.page.challenge().unwrap().is_some());
            let ctl = r.fake.ctl_file.clone();
            let page = ChromiumPage {
                notify: Arc::new(move |c: Cue| {
                    if c.activity == Activity::WaitingForYou {
                        std::fs::write(&ctl, "close-window\n").unwrap();
                    }
                }),
                ..page_with(&r, Lease::default())
            };
            let started = Instant::now();
            assert_eq!(page.hand_off_challenges(&s), Err(BrowserError::Closed));
            assert!(started.elapsed() < Duration::from_secs(10), "no wait for the user's check");
            r.renderers.kill_all();
        }

        #[test]
        fn stop_while_the_browser_starts_ends_it_once_it_started() {
            let r = rig(Mode::Obey, 1);
            // A browser that starts only when the test lets it.
            let go = r._dir.path().join("go");
            let gated = r._dir.path().join("gated.sh");
            std::fs::write(
                &gated,
                format!(
                    "#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nexec '{}' \"$@\"\n",
                    go.display(),
                    r.fake.exe.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&gated, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            r.renderers.set_sources(Sources {
                configured: Some(gated),
                installed: None,
            });
            let renderers = r.renderers.clone();
            let start = std::thread::spawn(move || renderers.session_in("main", &Lease::default()).map(|s| s.pid()));
            assert_eq!(r.renderers.wait_for("main", SEEN, |p| p != Phase::None), Phase::Launching);
            assert!(r.renderers.kill("main"), "a starting browser counts as running");
            assert_eq!(r.renderers.lifecycle("main").phase, Phase::Closing);
            std::fs::write(&go, "").unwrap();
            assert!(start.join().unwrap().is_err(), "a stopped launch hands out no browser");
            assert_eq!(r.renderers.lifecycle("main").phase, Phase::Gone { reason: GoneReason::Ended });
            assert!(r.renderers.current("main").is_none());
        }

        #[test]
        fn quitting_closes_every_browser_even_one_the_user_kept() {
            let r = rig(Mode::Obey, 1);
            let (dir, pid) = research_step(&r);
            r.renderers.current("main").unwrap().handed_off.store(true, Ordering::SeqCst);
            assert_eq!(r.renderers.finish("main"), Finish::KeptOpen(TookPart::PageNeededThem));
            r.renderers.close_all(QUIT_GRACE);
            assert!(gone(pid));
            assert!(!dir.exists());
            assert!(r.renderers.current("main").is_none());
        }
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

    fn live_page() -> Option<Live> {
        chromium_driver::find_for_tests()?;
        let renderers = Arc::new(Renderers::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let page = ChromiumPage {
            renderers: renderers.clone(),
            owner: "main".into(),
            may_drive: Some(Box::new(|| true)),
            rules: None,
            product: "Debrief".into(),
            notify: Arc::new(move |c| s2.lock().unwrap().push(c)),
            lease: Lease::default(),
        };
        Some((renderers, page, seen))
    }

    #[test]
    fn reads_a_scripted_page_through_the_ports_with_nothing_of_ours_in_it() {
        let Some((renderers, page, seen)) = live_page() else { return };
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
        let Some((renderers, page, seen)) = live_page() else { return };
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
