# CI (Gitea Actions, act_runner)

- Workflows: `.gitea/workflows/build.yml` (push to main, PRs, nightly schedule, manual), `bench.yml` (imt-core changes + nightly), `fuzz.yml` (nightly, 4 shards, pinned nightly toolchain), `mutants.yml` (weekly + PRs).
- Installers (bundle job) only for `v*` tags, the nightly run and manual runs.
- Linux jobs: Docker containers (`catthehacker/ubuntu:act-latest`) on shared runners; caches via an external cache server that sometimes 401s/times out.
- Windows jobs: host mode on the developer's PC (C:\gitea-runner, service account = user, session 0, capacity 1). No actions/cache on Windows; persistent `%USERPROFILE%\.imt-ci\target-<job>` and npm cache; host Node must match app/.nvmrc; Defender exclusions on C:\gitea-runner and .imt-ci.
- Session-0 specifics: WebView2 needs an explicit `--user-data-dir` (msedgedriver's temp would be C:\Windows\SystemTemp); window close can stall after exit → exit watchdog ends the process; no taskbar → no tray.
- Smoke harness (`app/e2e-tauri/wdio.conf.ts`): free driver ports per run, waits for both tauri-driver and native driver ports, keeps IMT logs on failure (printed by a follow-up step).
- Logs via API need a token (`/actions/jobs/{id}/logs`); job ids come from `/actions/runs/{run}/jobs`.