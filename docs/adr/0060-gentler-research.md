# 60. Gentler research: pacing, a chat's cache and an opt-in research profile

- Status: Accepted (2026-10-24)
- Date: 2026-10-24
- Related: ADR 0043 (the renderer), ADR 0057 (the AI browser's life cycle), Paddock rules 11, 13, 21

## Context

Research met "verify you are human" checks often. Two causes were Debrief's: it searched and opened result pages back
to back (a fixed 1.5 s between pages, none between searches, paging on to the engine's next results pages until enough
results were read), and every browser started with an empty throwaway profile, so an engine asked for consent and
checks on every research. A research also fetched again what the same chat had just fetched.

Debrief never solves or bypasses a check. Pausing for the user (ADR 0057) stays the fallback.

## Decision

1. **Pacing** (`debrief_ai::browser::gentle::Pacing`, Settings ▸ Web browser, advanced). Before each search request
   and each result page load research waits a pause picked in `pauseMin..=pauseMax` (1.5–4 s by default), shown as the
   job's phase ("Pausing before the next search" / "… page"). Searches keep the pause after the app's last search
   (`SearchClock`, shared by all researches), so a research that follows another waits too; the first search of the app
   goes at once. One research makes at most `maxSearches` search requests (3 by default; the first results page and
   the engine's next ones). The clock and the random pick are injected (`ResearchEnv::now`, `sleep`, `jitter`): tests
   use a fake clock and no sleeps.
2. **One research at a time** stays the job queue's limit for `JobKind::Research` (app-wide 1). A waiting research
   says "Waiting for the current research" (`JobKind::waiting_message`), so the Jobs pane and a waiting chat turn show
   why.
3. **A chat's cache** (`ResearchCache`, one per chat in `BrowserHub`, forgotten with the chat; bounded). A research
   first uses what its chat already fetched: the same question and search text gets the earlier answer (no request);
   the same search text reuses the result links taken from its first results page (no search request); a result page
   read before is read from the cache (no page load, no pause). "Research this" for a new chat hands its cache to the
   chat it answered in. The model's `web_research` uses its chat's cache the same way.
4. **An opt-in research profile** (`browser::profile`, Settings ▸ Web browser ▸ Keep a research profile, off by
   default). On, `Renderers` starts the window's AI browser with `<data folder>/research-profile` (never the user's
   browser profile) and a capped disk cache (50 MB); off, every browser gets a throwaway profile as before.
   - *One browser per profile.* Chrome locks a profile, and research runs one at a time, so the only contention is
     another window's chat browser: it gets a throwaway profile, quietly. The hold is released when the browser is
     gone (`KeptHold`, dropped after the browser).
   - *Size limit* (500 MB by default). A larger profile is not used: research uses a throwaway one and says why.
   - *Damage.* `Local State` or `Default/Preferences` that is not a JSON object, or a browser that does not start with
     the profile, means a throwaway profile for that research; the reason is logged, told once in the answer, and shown
     in Settings until the profile is fine or cleared.
   - *Clear research profile* (Settings ▸ Web browser) removes it. It is refused with a plain reason while a research is
     running or waiting, or while a browser uses the profile. Clear is also the repair of a damaged or too large profile
     (rule 19).

## Consequences

- Research is slower by a few seconds per page; the job's phase says why. A chat that asks again is faster.
- With the profile on, cookies and history of the research pages stay on disk until cleared; the setting says so.
- The tests cover the pauses and the search cap with a fake clock, the cache, the queue's message, the profile reused
  across two researches, throwaway when off, Clear refused while in use or while a research runs, and the damaged,
  too large and failing-to-start fallbacks, all with the fake browser (no real browser, no window).
