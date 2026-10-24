//! Gentler research (ADR 0058): fewer requests to search engines and sites, so fewer CAPTCHAs. Nothing here solves or
//! bypasses a check; a page that needs the user still waits for them.
//!
//! - [`Pacing`]: a short, jittered pause before every search request and every result page load, and a cap on the
//!   search requests (results pages) of one research.
//! - [`SearchClock`]: when the app's last search request went out, so consecutive researches keep the pause too.
//! - [`ResearchCache`]: what one chat already fetched (the results of a search, the pages read, whole answers), reused
//!   before searching or loading again.
//!
//! Pure and bounded; the clock and the random pause are passed in by the caller.

use super::capture::PageCapture;
use super::research::ResearchReport;
use super::url::WebUrl;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The default pause range before a search request or a result page load.
pub const DEFAULT_PAUSE_MIN: Duration = Duration::from_millis(1500);
pub const DEFAULT_PAUSE_MAX: Duration = Duration::from_millis(4000);
/// The longest pause the settings accept.
pub const MAX_PAUSE: Duration = Duration::from_secs(30);
/// Search requests (results pages) one research may make by default, and the limits of the setting.
pub const DEFAULT_MAX_SEARCHES: u8 = 3;
pub const MIN_SEARCHES: u8 = 1;
pub const MAX_SEARCHES: u8 = 10;
/// What one chat's cache keeps (oldest dropped first).
const CACHED_SEARCHES: usize = 20;
const CACHED_PAGES: usize = 40;
const CACHED_ANSWERS: usize = 20;

/// How gently a research goes about the web (Settings ▸ Web browser).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacing {
    /// The pause before a search request or a page load is picked in `pause_min..=pause_max`.
    pub pause_min: Duration,
    pub pause_max: Duration,
    /// Search requests (the first results page and the engine's next ones) per research.
    pub max_searches: u8,
}

impl Default for Pacing {
    fn default() -> Self {
        Pacing {
            pause_min: DEFAULT_PAUSE_MIN,
            pause_max: DEFAULT_PAUSE_MAX,
            max_searches: DEFAULT_MAX_SEARCHES,
        }
    }
}

impl Pacing {
    /// The same pause every time (tests, live tests).
    pub fn fixed(pause: Duration) -> Pacing {
        Pacing {
            pause_min: pause,
            pause_max: pause,
            ..Pacing::default()
        }
    }

    /// Valid values: pauses within [`MAX_PAUSE`], `min <= max` (swapped when given the other way round), searches
    /// within [`MIN_SEARCHES`]..=[`MAX_SEARCHES`].
    pub fn new(pause_min: Duration, pause_max: Duration, max_searches: u8) -> Pacing {
        let (a, b) = (pause_min.min(MAX_PAUSE), pause_max.min(MAX_PAUSE));
        Pacing {
            pause_min: a.min(b),
            pause_max: a.max(b),
            max_searches: max_searches.clamp(MIN_SEARCHES, MAX_SEARCHES),
        }
    }
}

/// A pause in `min..=max` from `unit`, a number in `0.0..1.0` (the caller's random source; tests pass a fixed one).
pub fn pause_between(min: Duration, max: Duration, unit: f64) -> Duration {
    let unit = if unit.is_finite() { unit.clamp(0.0, 1.0) } else { 0.0 };
    min + (max.saturating_sub(min)).mul_f64(unit)
}

/// When the app's last search request went out (shared by every research, so a queued research keeps the pause too).
#[derive(Debug, Default)]
pub struct SearchClock(Mutex<Option<Instant>>);

impl SearchClock {
    /// How long to wait at `now` before the next search, for a pause of `gap` (zero when the last one is long enough ago).
    pub fn wait_for(&self, gap: Duration, now: Instant) -> Duration {
        match *lock(&self.0) {
            Some(last) => gap.saturating_sub(now.saturating_duration_since(last)),
            None => Duration::ZERO,
        }
    }

    /// A search request went out at `now`.
    pub fn record(&self, now: Instant) {
        *lock(&self.0) = Some(now);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The key of a search text or question: case and spacing do not matter.
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// What one chat already fetched from the web. Bounded; forgotten with the chat.
#[derive(Debug, Default)]
pub struct ResearchCache(Mutex<Cached>);

#[derive(Debug, Default)]
struct Cached {
    /// Search text → the result links research took from its first results page.
    searches: VecDeque<(String, Vec<(WebUrl, String)>)>,
    /// Address → the page as read (redacted, capped).
    pages: VecDeque<(String, PageCapture)>,
    /// (question, search text) → the answer.
    answers: VecDeque<((String, String), ResearchReport)>,
}

fn put<K: PartialEq, V>(q: &mut VecDeque<(K, V)>, key: K, value: V, cap: usize) {
    q.retain(|(k, _)| *k != key);
    q.push_back((key, value));
    while q.len() > cap {
        q.pop_front();
    }
}

impl ResearchCache {
    pub fn links(&self, query: &str) -> Option<Vec<(WebUrl, String)>> {
        let key = normalize(query);
        lock(&self.0).searches.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }

    pub fn put_links(&self, query: &str, links: Vec<(WebUrl, String)>) {
        put(&mut lock(&self.0).searches, normalize(query), links, CACHED_SEARCHES);
    }

    pub fn page(&self, url: &str) -> Option<PageCapture> {
        lock(&self.0).pages.iter().find(|(k, _)| k == url).map(|(_, v)| v.clone())
    }

    pub fn put_page(&self, url: &str, page: PageCapture) {
        put(&mut lock(&self.0).pages, url.to_string(), page, CACHED_PAGES);
    }

    pub fn answer(&self, question: &str, query: &str) -> Option<ResearchReport> {
        let key = (normalize(question), normalize(query));
        lock(&self.0).answers.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }

    pub fn put_answer(&self, question: &str, query: &str, report: ResearchReport) {
        put(
            &mut lock(&self.0).answers,
            (normalize(question), normalize(query)),
            report,
            CACHED_ANSWERS,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_is_kept_within_its_limits() {
        let p = Pacing::new(Duration::from_secs(5), Duration::from_secs(2), 0);
        assert_eq!(
            (p.pause_min, p.pause_max, p.max_searches),
            (Duration::from_secs(2), Duration::from_secs(5), 1)
        );
        let p = Pacing::new(Duration::from_secs(99), Duration::from_secs(99), 200);
        assert_eq!((p.pause_max, p.max_searches), (MAX_PAUSE, MAX_SEARCHES));
        assert_eq!(Pacing::default().pause_min, DEFAULT_PAUSE_MIN);
        assert_eq!(Pacing::fixed(Duration::ZERO).pause_max, Duration::ZERO);
    }

    #[test]
    fn a_pause_is_picked_within_the_range() {
        let (a, b) = (Duration::from_millis(1500), Duration::from_millis(4000));
        assert_eq!(pause_between(a, b, 0.0), a);
        assert_eq!(pause_between(a, b, 1.0), b);
        assert_eq!(pause_between(a, b, 0.5), Duration::from_millis(2750));
        assert_eq!(pause_between(a, b, f64::NAN), a);
        assert_eq!(pause_between(a, b, 7.0), b);
        assert_eq!(pause_between(b, a, 0.5), b, "an empty range is its start");
    }

    #[test]
    fn the_search_clock_waits_only_for_what_is_left_of_the_pause() {
        let c = SearchClock::default();
        let t = Instant::now();
        assert_eq!(
            c.wait_for(Duration::from_secs(3), t),
            Duration::ZERO,
            "the first search goes at once"
        );
        c.record(t);
        assert_eq!(
            c.wait_for(Duration::from_secs(3), t + Duration::from_secs(1)),
            Duration::from_secs(2)
        );
        assert_eq!(c.wait_for(Duration::from_secs(3), t + Duration::from_secs(9)), Duration::ZERO);
    }

    #[test]
    fn the_cache_matches_regardless_of_case_and_spacing_and_is_bounded() {
        let c = ResearchCache::default();
        let u = WebUrl::parse("https://example.org/a").unwrap();
        c.put_links("IIS  0x80070005", vec![(u.clone(), "A".into())]);
        assert_eq!(c.links("iis 0x80070005").unwrap()[0].0, u);
        assert!(c.links("other").is_none());
        for i in 0..CACHED_PAGES + 5 {
            c.put_page(
                &format!("https://e.org/{i}"),
                PageCapture {
                    title: String::new(),
                    url: String::new(),
                    text: i.to_string(),
                    truncated: false,
                    removed: vec![],
                },
            );
        }
        assert!(c.page("https://e.org/0").is_none(), "the oldest went");
        assert_eq!(
            c.page(&format!("https://e.org/{}", CACHED_PAGES + 4)).unwrap().text,
            (CACHED_PAGES + 4).to_string()
        );
        let report = ResearchReport {
            query: "q".into(),
            answer: "a".into(),
            read: vec![],
            skipped: vec![],
            no_results: None,
            results_page: None,
        };
        c.put_answer("Why?", "q", report.clone());
        assert_eq!(c.answer("why?", " Q "), Some(report));
        assert!(c.answer("why not?", "q").is_none());
    }
}
