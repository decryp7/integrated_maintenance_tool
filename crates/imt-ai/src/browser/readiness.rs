//! When a search results page is ready to be read. The app waits for the page load to finish (WebView2
//! `NavigationCompleted`), but results pages render their results by script after that, so research looks at the page
//! every [`LOOK_EVERY`] (with the results-page collector, which also reports `document.readyState` and how long the DOM
//! has been quiet) and reads it once [`readiness`] says so, or after [`LOOK_FOR`] at the latest. No "network idle"
//! heuristic: what counts is the page's own state and its DOM.

use super::results::{ReadyState, ResultsPage};
use std::time::Duration;

/// How often the results page is looked at while it renders.
pub const LOOK_EVERY: Duration = Duration::from_millis(250);
/// The longest research waits for the results page (a first visit can show a redirect or script check first).
pub const LOOK_FOR: Duration = Duration::from_secs(15);
/// How long the DOM must have been quiet (no nodes added or removed) once the results region has links.
pub const QUIET: Duration = Duration::from_millis(500);
/// How long a complete page without links in its results region must have been quiet before it is read anyway (a page
/// with no results, or one whose region the engine renamed: the rules then look at every link).
pub const QUIET_EMPTY: Duration = Duration::from_secs(2);

/// Looks at the results page at most (one every [`LOOK_EVERY`] for [`LOOK_FOR`]).
pub const fn max_looks() -> usize {
    (LOOK_FOR.as_millis() / LOOK_EVERY.as_millis()) as usize
}

/// Whether a results page can be read now, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// `document.readyState` is not `complete` yet.
    Loading,
    /// The results region has links, but the DOM is still changing.
    Changing,
    /// Complete, but no links in the results region yet.
    NoResultsYet,
}

/// Ready once the document is complete, the results region has links and the DOM has been quiet for [`QUIET`]; a
/// complete page without region links is ready after [`QUIET_EMPTY`] of quiet. Pure: the caller polls.
pub fn readiness(page: &ResultsPage) -> Readiness {
    if page.ready_state != ReadyState::Complete {
        return Readiness::Loading;
    }
    let quiet = Duration::from_millis(page.quiet_ms.unwrap_or(0));
    match page.links_in_region() {
        0 if quiet >= QUIET_EMPTY => Readiness::Ready,
        0 => Readiness::NoResultsYet,
        _ if quiet >= QUIET => Readiness::Ready,
        _ => Readiness::Changing,
    }
}

#[cfg(test)]
mod tests {
    use super::super::results::fixtures::link;
    use super::*;

    fn page(state: ReadyState, quiet_ms: Option<u64>, in_region: bool) -> ResultsPage {
        ResultsPage {
            url: "https://search.example/?q=x".into(),
            links: vec![
                link("https://search.example/settings", "Settings", false),
                link("https://answers.example/1", "Answer", in_region),
            ],
            region_found: true,
            ready_state: state,
            quiet_ms,
            ..ResultsPage::default()
        }
    }

    #[test]
    fn a_page_is_read_once_complete_with_region_links_and_quiet() {
        assert_eq!(readiness(&page(ReadyState::Complete, Some(500), true)), Readiness::Ready);
        assert_eq!(readiness(&page(ReadyState::Complete, Some(5_000), true)), Readiness::Ready);
        assert_eq!(
            readiness(&page(ReadyState::Complete, Some(499), true)),
            Readiness::Changing,
            "results still rendering"
        );
        assert_eq!(
            readiness(&page(ReadyState::Complete, None, true)),
            Readiness::Changing,
            "the first look installs the observer: not quiet yet"
        );
    }

    #[test]
    fn a_page_still_loading_is_never_read() {
        for state in [ReadyState::Loading, ReadyState::Interactive, ReadyState::Unknown] {
            assert_eq!(readiness(&page(state, Some(60_000), true)), Readiness::Loading, "{state:?}");
        }
    }

    #[test]
    fn a_page_without_region_links_is_read_after_a_longer_quiet() {
        assert_eq!(readiness(&page(ReadyState::Complete, Some(1_999), false)), Readiness::NoResultsYet);
        assert_eq!(readiness(&page(ReadyState::Complete, Some(2_000), false)), Readiness::Ready);
    }

    #[test]
    fn looks_poll_four_times_a_second_for_fifteen_seconds() {
        assert_eq!(max_looks(), 60);
        assert!(QUIET < QUIET_EMPTY && QUIET_EMPTY < LOOK_FOR);
    }
}
