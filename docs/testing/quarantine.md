# Quarantined and environment-bound tests

Tests that are skipped, or cannot run, in some environment, with the reason. They are test-only problems (none hides a
product defect that was found); the post-v1 test-architecture work removes them: keyboard-focus checks that do not
depend on the OS setting, and a plugin-sandbox test mode that works inside another sandbox.

| Test | Reason | Environment | Expires |
| --- | --- | --- | --- |
| `e2e/activity-bar.e2e.ts` "works from the keyboard alone..." | Tab does not reach buttons in macOS WebKit without the system "full keyboard access" setting | Playwright WebKit on macOS (skipped there; runs on Linux and Windows CI) | 2027-03-31 |
| `e2e/store-pane.e2e.ts` "Store pane: reports, maintenance and the read-only SQL console" | Same Tab focus behaviour in a confirmation dialog | Playwright WebKit on macOS (skipped there) | 2027-03-31 |
| Plugin process tests (`debrief-plugin-test` harness, `debrief-plugin-e2e`, `debrief-example-*`, `debrief-store-worker` worker tests) | The plugin child calls macOS `sandbox_init`, which fails when the test run is itself inside another macOS sandbox (an agent shell); the child exits at start | Local runs inside a macOS sandbox only; they pass on Linux and Windows CI | 2027-03-31 |
| `debrief-plugin-dotnet` `real_runtime` | Needs a real .NET 10 runtime | Machines without it | 2027-03-31 |

Every entry has an `Expires` date (YYYY-MM-DD, at most 6 months out): `scripts/ci/check-quarantine.mjs` fails the build once a date
has passed, so a quarantine is either fixed or consciously renewed here with a new date.

Flaky-by-load tests are not listed: they were fixed (explicit waits, a 60 s CI timeout) rather than quarantined.
