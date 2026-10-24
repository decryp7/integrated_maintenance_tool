# 57. The AI browser has one life cycle, owned in one place

- Status: Accepted (2026-10-24)
- Date: 2026-10-24
- Related: ADR 0043 (the renderer: a real Chromium driven over DevTools), ADR 0055 (service health), ADR 0060 (gentler
  research: pacing, a chat's cache, the opt-in research profile), Paddock rules 5,
  7, 11, 19 and 40

## Context

User report (macOS, 1.0.x DMG): *"If I close the research Chrome window, I cannot do research anymore."*

Root cause. `Renderers` cached one `Session` per Debrief window and reused it while `Chromium::is_running()` held,
which checked only that the process was alive and the DevTools socket open. On macOS, closing a browser's last
window does not quit the browser: the process and the socket stay, but the page target Debrief drove is destroyed.
Every later research got the stale session, and every call on its page failed ("the browser was closed" / "No
session with given id") until the user pressed Stop or restarted Debrief. Nothing noticed the end while the browser
was idle, so the "kept open" note stayed too, offering to close a window that was gone. A research that lost its
browser half-way skipped each remaining result with "did not load" and then answered "None of the result pages
answered the question" instead of saying what happened. Tests also showed a second, latent bug: throwaway profile
folders were named by the clock, which macOS reads in microseconds, so two launches at once could share a folder.

This is one instance of a recurring kind (rule 40): Debrief keeps a handle to something outside it (a process, a
connection, an OS window) and decides it is usable from a partial check, in several places, each with its own idea
of "running".

## Decision

1. **One owner, one state machine.** `browser::sessions::Renderers` alone holds each window's browser and its phase
   (`browser::lifecycle`): `none → launching → ready → closing → gone(reason)`, and `gone → launching` for the next
   use. Every change goes through one function that checks the step (`Phase::may_become`) and tells the listeners.
   The reasons are `windowClosed`, `exited`, `disconnected`, `ended` (Debrief ended it) and `launchFailed`.
2. **Every end is detected, by the driver.** `chromium_driver::Chromium::gone(page)` reports `Exited` (process
   wait), `Disconnected` (the DevTools socket ended) or `PageClosed` (the browser reported the page's session
   detached or its target destroyed or crashed; the connection tracks the ids it watches, not the bounded event
   log). A watcher thread per ready browser checks this every 200 ms and asks the browser for its targets every
   second (a close it did not report); every use checks it too. A page call or load wait on a closed page returns
   `Error::Closed` at once instead of waiting for its timeout.
3. **Gone means gone.** The transition to `gone` drops the "kept open" mark, ends what is left (a browser without a
   window is closed, then killed), removes the throwaway profile (the opt-in research profile of ADR 0060 is kept, and
   released for the next browser) and tells the window (`lifecycle` event). The next
   use launches a fresh browser.
4. **A run keeps to its browser.** Research holds a `Lease`: the first use pins the browser generation, and once it
   is gone the run fails with `BrowserError::Closed` instead of opening another window behind the user's back.
   Research turns that into one plain message with Retry (the job's retry), and stops trying further results.
5. **Launch outside the lock.** `launching` is a phase, not a held mutex, so Stop, Close and other windows' queries
   are never blocked by a slow start; Stop during `launching` ends the browser as soon as it started.
6. **The end of a research is decided here too.** `Renderers::finish` runs when a research ends (done, cancelled or
   failed, from *Research this* or the model's `web_research` alike): it closes the browser (`closing → gone(ended)`:
   `Browser.close`, then kill, profile removed) unless the user took part in it, and then marks it `kept`. The user took
   part when a page needed them, the page shows another address than the AI's last navigation, or the browser has more
   than one page. The driver drives the page the browser opened with (`Chromium::start_page`), so a second page is
   always the user's. The decision and its reason are logged in one info line (no addresses). The window learns it
   only from the `lifecycle` event (`kept`); there is no separate "kept" event. A new research in a kept browser starts
   afresh: the earlier part no longer counts and the `kept` mark goes.
7. **Health reads the same state.** `Renderers::lifecycle(owner)`, `snapshot()` and `listen()` are what the service
   registry of ADR 0055 reads; it derives nothing from error text.

## Consequences

- Closing the window, quitting the browser, a crash or a lost connection are all the same transition, covered by
  tests with the fake browser (`chromium_driver::fake::UserAction`): idle and kept open, between research steps, in
  the middle of a page load, during a challenge wait, and Stop while starting. No real browser and no window.
- The wire contract gains `{ type: 'lifecycle', state, reason?, kept }` on `debrief://browser`; the chat shows its
  "left open because you used it" note with Close it / Keep it while the browser is ready and kept, and clears it
  otherwise. The former `researchBrowserKept` event is removed.
- 1.1.0 regression: research never closed its browser, because the driver opened a second page beside the browser's
  start page and "more than one page" therefore always meant "the user took part". The fake browser now starts with
  its start page and adds one per `Target.createTarget`, as Chrome does, so the tests see that case.
- Other supervised children (AI worker, plugins, collectors) are checked for the same kind; where they cache a handle,
  they should move to the same shape (one owner, explicit phase, detection by wait + disconnect, fresh start).
