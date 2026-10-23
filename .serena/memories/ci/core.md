# CI (Gitea Actions, act_runner)

- Workflows: `.gitea/workflows/build.yml` (push to main, PRs, nightly schedule, manual), `bench.yml` (debrief-core changes + nightly), `fuzz.yml` (nightly, 4 shards, pinned nightly toolchain), `mutants.yml` (weekly + PRs).
- Installers (bundle job) only for `v*` tags, the nightly run and manual runs.
- Linux jobs: Docker containers (`catthehacker/ubuntu:act-latest`) on shared runners; caches via an external cache server that sometimes 401s/times out.
- Windows jobs: host mode on the developer's PC, label `windows-latest`, capacity 1. The runner is NOT a service: scheduled task `GiteaRunnerDesktop` starts `C:\gitea-runner\run-desktop.ps1` at the user's logon in the interactive desktop session (WebView2 is unsupported in session 0); jobs only run while the user is logged on. `C:\gitea-runner\update-runner.ps1` (admin) sets this up and self-updates daily at 03:00 (skips while a job runs). Log: `C:\gitea-runner\logs\runner-desktop.log`.
- Windows job setup: no actions/cache, no setup-node (host Node must match app/.nvmrc); persistent `%USERPROFILE%\.debrief-ci\target-<job>` and npm cache; Defender exclusions on C:\gitea-runner and .debrief-ci.
- Leftover session-0 workarounds (harmless): `--user-data-dir` capability in the smoke harness, tray skipped without a taskbar, exit watchdog (also protects real users).
- Smoke harness (`app/e2e-tauri/wdio.conf.ts`, `support/app.ts`): free driver ports per run, waits for both driver ports, `quitApp` closes extra windows via Debrief's menu (never the driver's closeWindow), keeps Debrief logs on failure (printed by a follow-up step).
- Logs via API need a token (`/actions/jobs/{id}/logs`); job ids come from `/actions/runs/{run}/jobs`.