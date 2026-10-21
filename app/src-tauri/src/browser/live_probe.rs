//! Live test of research's results-page step against the real search engines, in the real page webview (WebView2 on
//! Windows, WebKitGTK on Linux), with the real collector script and the real rules (`research::search`,
//! `research::read_results`). Compiled only with the `live-search-probe` feature, which only debug builds accept
//! (`npm run e2e:tauri:build:live`): shipped builds have no trace of it. It adds no command, no event and no
//! capability: when `IMT_E2E_LIVE_SEARCH_OUT` names an absolute folder at start-up, the app searches once per built-in
//! engine in the main window's browser and writes `live-search.json` (and a screenshot per engine on Windows) there.
//! The WebdriverIO spec `app/e2e-tauri/live` starts the app, waits for the report and judges it; see
//! docs/development.md. The queries are neutral (never user data), and the report keeps page addresses without their
//! query string.

#[cfg(not(debug_assertions))]
compile_error!("the live-search-probe feature is for debug end-to-end builds only, never for shipped builds");

use super::page::TauriPage;
use super::research::ReadOnlyDriver;
use super::{ActivityLog, BrowserState};
use imt_ai::browser::research::{self, ResearchEnv, ResultsOutcome, ResultsRead};
use imt_ai::browser::{AgentLimits, PageDiagnostics, PageKind, SearchEngine, SearchSettings};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

/// The folder the report goes to (absolute).
pub const OUT_VAR: &str = "IMT_E2E_LIVE_SEARCH_OUT";
/// Replaces the default query (at most [`MAX_QUERY`] printable characters).
pub const QUERY_VAR: &str = "IMT_E2E_LIVE_SEARCH_QUERY";
/// Limits the probe to some engines (comma-separated ids, e.g. `google,bing`).
pub const ENGINES_VAR: &str = "IMT_E2E_LIVE_SEARCH_ENGINES";
/// A neutral search every engine has results for.
pub const DEFAULT_QUERY: &str = "System.TimeoutException";
pub const REPORT: &str = "live-search.json";
const MAX_QUERY: usize = 100;
/// Result links an engine must give for the check to pass.
pub const MIN_LINKS: usize = 3;
/// The owner window whose browser is used.
const OWNER: &str = crate::windows::MAIN;
/// Pause between two engines (a human pace).
const PACE: Duration = Duration::from_secs(3);

static SLEEP: fn(Duration) = std::thread::sleep;
static NOW: fn() -> Instant = Instant::now;

/// How one engine did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// At least [`MIN_LINKS`] result links.
    Pass,
    /// A consent, human-check or redirect page that research hands to the user: recognised, but no links checked.
    Warn,
    Fail,
}

/// The verdict on what research read and on opening its first result (`Ok(site)` where it was shown, or why not), and a
/// line saying why.
pub fn verdict(outcome: &ResultsOutcome, first: Option<&Result<String, String>>) -> (Verdict, String) {
    match outcome {
        ResultsOutcome::Links(l) if l.len() >= MIN_LINKS => match first {
            Some(Ok(site)) => (Verdict::Pass, format!("{} result links; the first one opened on {site}", l.len())),
            Some(Err(why)) => (Verdict::Fail, format!("{} result links, but the first one {why}", l.len())),
            None => (Verdict::Fail, format!("{} result links; the first one was not opened", l.len())),
        },
        ResultsOutcome::Links(l) => (
            Verdict::Fail,
            format!("only {} result links (at least {MIN_LINKS} expected)", l.len()),
        ),
        ResultsOutcome::NoLinks => (Verdict::Fail, "the results page showed no result links the rules recognise".into()),
        ResultsOutcome::HandOff(kind) => (Verdict::Warn, format!("handed to the user: {}", describe(*kind))),
        ResultsOutcome::NotShown => (Verdict::Fail, "the results page never showed".into()),
    }
}

fn describe(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Consent => "a consent page",
        PageKind::Captcha => "a human check",
        PageKind::Redirect => "a redirect page",
        PageKind::Results => "a results page",
    }
}

/// One engine's line of the report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineReport {
    pub engine: &'static str,
    pub label: &'static str,
    pub verdict: Verdict,
    pub note: String,
    pub links: usize,
    /// The result links' sites (never their addresses).
    pub result_sites: Vec<String>,
    pub looks: usize,
    pub ready: bool,
    pub elapsed_ms: u64,
    pub page: Option<PageDiagnostics>,
    pub screenshot: Option<String>,
    pub screenshot_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    query: &'a str,
    platform: &'static str,
    done: bool,
    engines: &'a [EngineReport],
}

/// The built-in engines (every one but Custom), or those `only` lists by id (comma-separated; unknown ids are ignored).
pub fn selected(only: Option<&str>) -> Vec<SearchEngine> {
    let all = SearchEngine::ALL.into_iter().filter(|e| *e != SearchEngine::Custom);
    match only.map(str::trim).filter(|s| !s.is_empty()) {
        None => all.collect(),
        Some(list) => all.filter(|e| list.split(',').any(|id| id.trim() == e.id())).collect(),
    }
}

/// The query: the variable's (trimmed, printable, at most [`MAX_QUERY`] characters) or [`DEFAULT_QUERY`].
pub fn query_from(var: Option<String>) -> Result<String, String> {
    match var {
        None => Ok(DEFAULT_QUERY.into()),
        Some(q) => {
            let q = q.trim();
            if q.is_empty() || q.chars().count() > MAX_QUERY || q.chars().any(char::is_control) {
                Err(format!("{QUERY_VAR} must be 1 to {MAX_QUERY} printable characters"))
            } else {
                Ok(q.to_string())
            }
        }
    }
}

/// Starts the probe when [`OUT_VAR`] is set (call from `browser::init`). Runs on its own thread.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let Some(out) = std::env::var_os(OUT_VAR).map(PathBuf::from) else {
        return;
    };
    if !out.is_absolute() {
        log::error!("{OUT_VAR} must be an absolute folder: the live search probe does not run");
        return;
    }
    let query = match query_from(std::env::var(QUERY_VAR).ok()) {
        Ok(q) => q,
        Err(e) => {
            log::error!("{e}: the live search probe does not run");
            return;
        }
    };
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = run(&app, &out, &query) {
            log::error!("The live search probe failed: {e}");
        }
    });
}

fn run<R: Runtime>(app: &AppHandle<R>, out: &Path, query: &str) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    // The main window and its UI first (the Browser panel places the page webview).
    let deadline = Instant::now() + Duration::from_secs(90);
    while app.get_webview_window(OWNER).is_none_or(|w| !w.is_visible().unwrap_or(false)) {
        if Instant::now() > deadline {
            return Err("the main window did not show".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    std::thread::sleep(Duration::from_secs(5));
    // msedgedriver drives the main webview through WEBVIEW2_* variables, and WebView2 applies them to every environment
    // the process creates: the page webview's own environment (its own profile and arguments) then disagreed with them and
    // failed to start (0x8007139F). The main webview is running already; a user's IMT has none of these variables.
    let driver_vars: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.to_ascii_uppercase().starts_with("WEBVIEW2_"))
        .collect();
    for k in &driver_vars {
        std::env::remove_var(k);
    }
    log::info!("Live search probe: removed the WebDriver's variables {driver_vars:?}");
    let only = std::env::var(ENGINES_VAR).ok();
    let mut engines = Vec::new();
    for (i, engine) in selected(only.as_deref()).into_iter().enumerate() {
        if i > 0 {
            std::thread::sleep(PACE);
        }
        engines.push(probe(app, engine, query, out));
        write_report(out, query, false, &engines)?;
    }
    write_report(out, query, true, &engines)
}

fn write_report(out: &Path, query: &str, done: bool, engines: &[EngineReport]) -> Result<(), String> {
    let report = Report {
        query,
        platform: std::env::consts::OS,
        done,
        engines,
    };
    let json = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    let tmp = out.join(format!("{REPORT}.tmp"));
    std::fs::write(&tmp, json).map_err(|e| format!("cannot write the report: {e}"))?;
    std::fs::rename(&tmp, out.join(REPORT)).map_err(|e| format!("cannot write the report: {e}"))
}

/// One engine: search, read the results page as research does, take a screenshot of it, open the first result.
fn probe<R: Runtime>(app: &AppHandle<R>, engine: SearchEngine, query: &str, out: &Path) -> EngineReport {
    let started = Instant::now();
    let file = format!("live-{}.png", engine.id());
    let mut shot = Err("no screenshot taken".to_string());
    let read = read(app, engine, query, &mut || shot = screenshot(app, &out.join(&file)));
    let (verdict, note, read) = match read {
        Ok((r, first)) => {
            let (v, n) = verdict(&r.outcome, first.as_ref());
            (v, n, Some(r))
        }
        Err(e) => (Verdict::Fail, e, None),
    };
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (screenshot, screenshot_error) = match shot {
        Ok(()) => (Some(file), None),
        Err(e) => (None, Some(e)),
    };
    let links: Vec<String> = match read.as_ref().map(|r| &r.outcome) {
        Some(ResultsOutcome::Links(l)) => l.iter().map(|(u, _)| u.site().to_string()).collect(),
        _ => vec![],
    };
    EngineReport {
        engine: engine.id(),
        label: engine.label(),
        verdict,
        note,
        links: links.len(),
        result_sites: links,
        looks: read.as_ref().map_or(0, |r| r.looks),
        ready: read.as_ref().is_some_and(|r| r.ready),
        elapsed_ms,
        page: read.and_then(|r| r.page).map(|p| p.diagnostics()),
        screenshot,
        screenshot_error,
    }
}

/// The search, the results page and its first result, through the same ports and functions research uses, while the
/// probe drives the main window's browser (read-only, like research). `results_shown` runs while the results page shows.
/// The first result: `Ok(site)` it was shown on, or why not.
fn read<R: Runtime>(
    app: &AppHandle<R>,
    engine: SearchEngine,
    query: &str,
    results_shown: &mut dyn FnMut(),
) -> Result<(ResultsRead, Option<Result<String, String>>), String> {
    let hub = app.try_state::<BrowserState>().ok_or("the browser is not available")?.0.clone();
    let key = "live-search-probe".to_string();
    let control = hub.chat(OWNER, &key);
    control.set_agent(true, AgentLimits::default());
    if !hub.begin_driving(OWNER, &key) {
        hub.forget_chat(OWNER, &key);
        return Err("another chat drives the browser".into());
    }
    super::driving::refresh(app, OWNER);
    let page = |chat: Option<String>| TauriPage {
        app: app.clone(),
        owner: OWNER.to_string(),
        chat,
    };
    let reader = page(None);
    let driver = ReadOnlyDriver {
        page: page(Some(key.clone())),
        control,
    };
    let log = ActivityLog { window: OWNER.into() };
    let cancel = AtomicBool::new(false);
    let env = ResearchEnv {
        reader: &reader,
        results: &reader,
        driver: &driver,
        search: SearchSettings { engine, custom: None },
        log: &log,
        id: key.clone(),
        cancel: &cancel,
        pace: research::PAGE_PACE,
        total: research::TOTAL_TIME,
        sleep: &SLEEP,
        now: &NOW,
    };
    let r = research::search(&env, query).and_then(|url| {
        let read = research::read_results(&env, &url, 10)?;
        results_shown();
        let first = match &read.outcome {
            ResultsOutcome::Links(links) => match links.first() {
                Some((link, _)) => {
                    std::thread::sleep(research::PAGE_PACE);
                    Some(research::open_result(&env, link, &url, 1)?.map(|o| o.url.site().to_string()))
                }
                None => None,
            },
            _ => None,
        };
        Ok((read, first))
    });
    hub.end_driving(OWNER, &key);
    hub.forget_chat(OWNER, &key);
    super::driving::refresh(app, OWNER);
    r.map_err(|e| e.to_string())
}

/// A PNG of the page webview (WebView2's DevTools screenshot).
#[cfg(windows)]
fn screenshot<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    use base64::Engine as _;
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    let w = super::view::page_webview(app, OWNER).ok_or("the browser is not open")?;
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<String, String>>(1);
    let failed = tx.clone();
    w.with_webview(move |platform| {
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, json| {
            let _ = tx.try_send(hr.map(|()| json).map_err(|e| e.to_string()));
            Ok(())
        }));
        // SAFETY: COM calls on the webview's own controller, on the thread `with_webview` runs them on (the UI thread).
        let r = unsafe {
            platform.controller().CoreWebView2().and_then(|core| {
                core.CallDevToolsProtocolMethod(
                    &HSTRING::from("Page.captureScreenshot"),
                    &HSTRING::from(r#"{"format":"png"}"#),
                    &handler,
                )
            })
        };
        if let Err(e) = r {
            let _ = failed.try_send(Err(e.to_string()));
        }
    })
    .map_err(|e| e.to_string())?;
    let json = rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "no screenshot in time".to_string())??;
    let data = serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_str()).map(str::to_string))
        .ok_or("the screenshot reply has no image")?;
    let png = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn screenshot<R: Runtime>(_app: &AppHandle<R>, _path: &Path) -> Result<(), String> {
    Err("screenshots of the page webview are taken on Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::browser::WebUrl;

    fn links(n: usize) -> ResultsOutcome {
        ResultsOutcome::Links(
            (0..n)
                .map(|i| (WebUrl::parse(&format!("https://r{i}.example/")).unwrap(), "r".into()))
                .collect(),
        )
    }

    #[test]
    fn three_result_links_pass_hand_offs_warn_anything_else_fails() {
        let opened = Ok("learn.example.org".to_string());
        assert_eq!(verdict(&links(3), Some(&opened)).0, Verdict::Pass);
        let (v, note) = verdict(&links(3), Some(&Err("did not load (timeout)".into())));
        assert_eq!(v, Verdict::Fail, "the first result must open too");
        assert!(note.ends_with("the first one did not load (timeout)"), "{note}");
        assert_eq!(verdict(&links(3), None).0, Verdict::Fail);
        assert_eq!(verdict(&links(2), Some(&opened)).0, Verdict::Fail);
        assert_eq!(verdict(&ResultsOutcome::NoLinks, None).0, Verdict::Fail);
        assert_eq!(verdict(&ResultsOutcome::NotShown, None).0, Verdict::Fail);
        for kind in [PageKind::Consent, PageKind::Captcha, PageKind::Redirect] {
            let (v, note) = verdict(&ResultsOutcome::HandOff(kind), None);
            assert_eq!(v, Verdict::Warn);
            assert!(note.starts_with("handed to the user"), "{note}");
        }
    }

    #[test]
    fn every_builtin_engine_is_probed_unless_some_are_chosen() {
        use SearchEngine::*;
        assert_eq!(selected(None), [Google, DuckDuckGo, Bing]);
        assert_eq!(selected(Some(" ")), selected(None));
        assert_eq!(selected(Some("bing, google,custom,yahoo")), [Google, Bing]);
    }

    #[test]
    fn the_query_is_neutral_by_default_and_checked_when_given() {
        assert_eq!(query_from(None).unwrap(), DEFAULT_QUERY);
        assert_eq!(
            query_from(Some("  HTTP 504 Gateway Timeout ".into())).unwrap(),
            "HTTP 504 Gateway Timeout"
        );
        assert!(query_from(Some(" ".into())).is_err());
        assert!(query_from(Some("a\nb".into())).is_err());
        assert!(query_from(Some("x".repeat(MAX_QUERY + 1))).is_err());
    }
}
