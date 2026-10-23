//! Nightly and manual live test of research's results-page step against the real search engines, in a real Chromium
//! (ADR 0043; it replaces the web view probe). Ignored by default: run it with
//! `DEBRIEF_LIVE_SEARCH=1 cargo test -p debrief-app --lib live_search -- --ignored --nocapture`. It uses the same ports and
//! functions research does (`research::search`, `research::read_results`, `research::open_result`) through
//! [`ChromiumPage`], once per built-in engine, and fails with diagnostics (page kind, link counts, ready state, a
//! screenshot) unless an engine gives at least [`MIN_LINKS`] result links or a recognised consent/CAPTCHA hand-off.
//! `DEBRIEF_E2E_LIVE_SEARCH_OUT` names an absolute folder for `live-search.json` and the screenshots; the queries are
//! neutral (never user data) and the report keeps page addresses without their query string.

use super::renderer::{ChromiumPage, Renderers};
use super::research::ReadOnlyDriver;
use debrief_ai::browser::research::{self, ResearchEnv, ResultsOutcome, ResultsRead};
use debrief_ai::browser::{AgentLimits, ChatBrowser, PageDiagnostics, PageKind, SearchEngine, SearchSettings};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const OUT_VAR: &str = "DEBRIEF_E2E_LIVE_SEARCH_OUT";
pub const QUERY_VAR: &str = "DEBRIEF_E2E_LIVE_SEARCH_QUERY";
pub const ENGINES_VAR: &str = "DEBRIEF_E2E_LIVE_SEARCH_ENGINES";
pub const DEFAULT_QUERY: &str = "System.TimeoutException";
pub const REPORT: &str = "live-search.json";
const MAX_QUERY: usize = 100;
pub const MIN_LINKS: usize = 3;
const PACE: Duration = Duration::from_secs(3);

static SLEEP: fn(Duration) = std::thread::sleep;
static NOW: fn() -> Instant = Instant::now;

/// How one engine did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Pass,
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
        ResultsOutcome::NoLinks => (Verdict::Fail, "the results page showed no candidate result links".into()),
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

struct NoLog;
impl debrief_ai::browser::ActionLog for NoLog {
    fn record(&self, _: &debrief_ai::browser::ActionEntry) {}
}

fn screenshot(renderers: &Renderers, path: &Path) -> Result<(), String> {
    use base64::Engine as _;
    let s = renderers.current("live").ok_or("the browser is not open")?;
    let v = s.page().screenshot_png().map_err(|e| e.to_string())?;
    let png = base64::engine::general_purpose::STANDARD.decode(v).map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| e.to_string())
}

fn read(
    renderers: &Arc<Renderers>,
    engine: SearchEngine,
    query: &str,
    results_shown: &mut dyn FnMut(),
) -> Result<(ResultsRead, Option<Result<String, String>>), String> {
    let control = Arc::new(ChatBrowser::new(AgentLimits::default()));
    control.set_agent(true, AgentLimits::default());
    let page = |drive: bool| ChromiumPage {
        renderers: renderers.clone(),
        owner: "live".into(),
        may_drive: drive.then(|| Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>),
        rules: Some(control.clone()),
        product: "Debrief".into(),
        notify: Arc::new(|_| {}),
    };
    let reader = page(false);
    let driver = ReadOnlyDriver {
        page: page(true),
        control: control.clone(),
    };
    let cancel = AtomicBool::new(false);
    let env = ResearchEnv {
        reader: &reader,
        results: &reader,
        driver: &driver,
        search: SearchSettings { engine, custom: None },
        log: &NoLog,
        id: "live".into(),
        cancel: &cancel,
        pace: research::PAGE_PACE,
        total: research::TOTAL_TIME,
        sleep: &SLEEP,
        now: &NOW,
    };
    research::search(&env, query)
        .and_then(|url| {
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
        })
        .map_err(|e| e.to_string())
}

fn probe(renderers: &Arc<Renderers>, engine: SearchEngine, query: &str, out: &Path) -> EngineReport {
    let started = Instant::now();
    let file = format!("live-{}.png", engine.id());
    let mut shot = Err("no screenshot taken".to_string());
    let read = read(renderers, engine, query, &mut || shot = screenshot(renderers, &out.join(&file)));
    let (verdict, note, read) = match read {
        Ok((r, first)) => {
            let (v, n) = verdict(&r.outcome, first.as_ref());
            (v, n, Some(r))
        }
        // A page that waits for a person (consent, CAPTCHA) is the recognised hand-off, not a failure.
        Err(e) if e.contains("still needs you") => (Verdict::Warn, format!("handed to the user: {e}"), None),
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

/// Every built-in engine against the real service; needs a Chromium-based browser and the network.
#[test]
#[ignore = "live: needs a browser and the network (nightly / manual)"]
fn live_search_every_engine_gives_results_or_a_recognised_hand_off() {
    if std::env::var("DEBRIEF_LIVE_SEARCH").is_err() {
        eprintln!("set DEBRIEF_LIVE_SEARCH=1 to run the live search test");
        return;
    }
    let out = std::env::var_os(OUT_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("debrief-live-search"));
    assert!(out.is_absolute(), "{OUT_VAR} must be an absolute folder");
    std::fs::create_dir_all(&out).unwrap();
    let query = query_from(std::env::var(QUERY_VAR).ok()).unwrap();
    let renderers = Arc::new(Renderers::default());
    let mut engines = Vec::new();
    for (i, engine) in selected(std::env::var(ENGINES_VAR).ok().as_deref()).into_iter().enumerate() {
        if i > 0 {
            std::thread::sleep(PACE);
        }
        engines.push(probe(&renderers, engine, &query, &out));
    }
    renderers.kill_all();
    let report = Report {
        query: &query,
        platform: std::env::consts::OS,
        done: true,
        engines: &engines,
    };
    std::fs::write(out.join(REPORT), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let failed: Vec<_> = engines
        .iter()
        .filter(|e| e.verdict == Verdict::Fail)
        .map(|e| format!("{}: {}", e.label, e.note))
        .collect();
    assert!(failed.is_empty(), "live search failed: {failed:?} (report in {})", out.display());
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_ai::browser::WebUrl;

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
