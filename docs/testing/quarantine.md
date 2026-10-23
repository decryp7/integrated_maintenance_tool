# Quarantined and environment-bound tests

Tests that are skipped, or cannot run, in some environment, with the reason. They are test-only problems (none hides a
product defect that was found); the post-v1 test-architecture work removes them: keyboard-focus checks that do not
depend on the OS setting, and a plugin-sandbox test mode that works inside another sandbox.

| Test | Reason | Environment |
| --- | --- | --- |
| `e2e/activity-bar.e2e.ts` "works from the keyboard alone..." | Tab does not reach buttons in macOS WebKit without the system "full keyboard access" setting | Playwright WebKit on macOS (skipped there; runs on Linux and Windows CI) |
| `e2e/store-pane.e2e.ts` "Store pane: reports, maintenance and the read-only SQL console" | Same Tab focus behaviour in a confirmation dialog | Playwright WebKit on macOS (skipped there) |
| Plugin process tests (`debrief-plugin-test` harness, `debrief-plugin-e2e`, `debrief-example-*`, `debrief-store-worker` worker tests) | The plugin child calls macOS `sandbox_init`, which fails when the test run is itself inside another macOS sandbox (an agent shell); the child exits at start | Local runs inside a macOS sandbox only; they pass on Linux and Windows CI |
| `debrief-plugin-dotnet` `real_runtime` | Needs a real .NET 10 runtime | Machines without it |

Flaky-by-load tests are not listed: they were fixed (explicit waits, a 60 s CI timeout) rather than quarantined.
