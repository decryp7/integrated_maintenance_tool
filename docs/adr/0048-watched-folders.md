# 48. Watched folders

- Status: accepted (2026-10-22); the headless driver (`imt watch`) is built, the app's settings UI and desktop
  notifications are not yet
- Date: 2026-10-22
- Related: ADR 0012 (jobs and notifications), ADR 0039 (known issues), ADR 0044 (command line), ADR 0047

## Context

Support engineers and CI want files that drop into a folder imported without clicking, and to hear when a known issue
appears. The hard part is not noticing files but not reading them too early (a copy in progress), not twice, and not
forever (a locked or huge file).

## Decision

- **Polling, not OS events.** A scan every few seconds works on network shares, behaves the same on every platform
  and needs no new dependency. Scans are bounded (50,000 entries, 16 levels, no symbolic links).
- **A pure state machine.** `imt_core::watch::WatchState` takes folder snapshots and a clock and returns what is ready:
  a file must keep the same size and modification time for the debounce (2 s default); temporary names (`.part`,
  `.tmp`, `.crdownload`, `~$`, dotfiles) and empty files are ignored; a vanished file is forgotten; a failure is retried
  with doubling delays and given up after four attempts (a changed file is tried again); files over 4 GiB are reported
  once; a known file that changed is *refreshed*, a new one *opened*; the pending list is bounded (10,000). Every one of
  these is a unit test with a fake clock.
- **Notifications are data.** `WatchState::fresh_matches` returns the known issues not announced before or with more
  events than announced; the driver shows them (a line or a JSON object). The app will route the same values to the
  notification centre (ADR 0012).
- **Drivers import through the ports** (`Ingest::open` / `refresh`, one file at a time so a failure is attributed to
  its file) and match known issues through `match_known_issues`. `imt watch` is the first driver; `--once` settles a folder
  and exits, which is how it is tested.

## Consequences

- A growing log is refreshed whenever it settles, not tailed; live tail (ADR 0037) remains the tool for that.
- The desktop app needs a driver of its own (settings for the folders, a background task per project, notification
  centre entries); it reuses `WatchState` unchanged.
