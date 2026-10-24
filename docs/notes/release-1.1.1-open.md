# Release 1.1.1: open items (paused 2026-10-24)

Branch `fix/release-1.1.0` (from 58610259) fixes three of the four failures of tag run 6272: the third-party notices,
the Linux tray panic (GTK not initialised under the mock runtime) and the real-worker health test that rebuilt the
store worker inside the test. Still open:

## 1. macOS store worker query memory (`debrief-store-worker` `tests/worker.rs`, `QUERY_BUDGET_MB` = 550 on macOS)

CI measured `querying: 570 MB` on the macOS host runner under load; locally a run at load ~17 reached 552 MB.

Interleaved measurements on the Mac mini (querying peak, MB; each pair run back to back):

| build | querying | HEAD (branch) at the same time |
|---|---|---|
| v1.0.1 | 451-482 (mean ~467) | 503-533 (mean ~521) |
| 37dea417 (1.0.2 merge of fix/import-large-folder) | 464-499 (~481) | 487-543 (~515) |
| 5ceb1053 (1.0.2 merge of fix/long-operations) | 470-493 (~480) | 505-535 (~518) |
| 9e4369c2 (1.0.2 head) | 477-511 (~494) | 484-531 (~504) |

So about +40-50 MB since v1.0.1, idle about the same (retained after import and indexing). Most of it lies after
5ceb1053; 9e4369c2 is only ~10 MB under HEAD (one sample set, so check again). Next candidates, first-parent order:
482d9d28 (auto-hide), 6807089b (search "Did you mean" from find_values), f7e0705d (CLI progress), then the 1.1
commits after 9e4369c2 if 9e4369c2 turns out lower than measured.

Ruled out by A/B at HEAD (no measurable change): the streaming import vs the old `find_files` + `import_files_parallel`
path, recording source fingerprints, a one-thread full-text docs pool, one folder walker, the import dispatcher off the
parse pool, in-flight files = parse threads. Non-interleaved bisects are misleading (load moves the numbers by
±30 MB): always interleave.

Bisect continued (6807089b: ~483 vs ~524): the commits from there to 9e4369c2 do not touch the worker, and every step
lies within the noise, so no single commit carries the increase. Decision for 1.1.1: the features' cost; the macOS
`QUERY_BUDGET_MB` is 630 (570 MB on CI under load plus 10%). Follow-up for 1.2: find what the extra memory is
(allocator statistics or a heap profile of the worker at idle) and bring the budget back down.

Original question: a leak or retention to fix (with a test that the memory is released), or a feature cost (then
the budget comment records the measurement and the budget becomes measured max + ~10%).

### How to measure

Needs a second worktree and target (`git worktree add --detach <dir> v1.0.1`, target seeded with `cp -c -R`), the
libduckdb environment from `crates/debrief-store-duck/scripts/fetch-libduckdb.sh`, and a 1-minute load at most 16.
`ab.sh <rev> [n]` builds `<rev>` in the base worktree and prints n interleaved pairs (rev, then the branch):

```bash
#!/bin/bash
# ab.sh <rev> [n]: interleaved querying-peak measurements of <rev> (base worktree) against the branch (HEAD worktree)
source "$ENV_FILE"   # CARGO_TARGET_DIR, RUSTC_WRAPPER=sccache, CARGO_BUILD_JOBS=2, DUCKDB_*; cd to the branch worktree
B=<base worktree>; BT=<its target dir>
(cd $B && git checkout -q --detach "$1" && CARGO_TARGET_DIR=$BT cargo build -q -p debrief-plugins && CARGO_TARGET_DIR=$BT cargo nextest run --no-run -p debrief-store-worker >/dev/null 2>&1) || { echo buildfail; exit 1; }
m() { cargo nextest run -p debrief-store-worker --no-capture -E 'test(=imports_and_queries_within_the_budgets)' 2>&1 | grep -E "worker: imported" | sed -E 's/.*idle ([0-9]+) MB, querying ([0-9]+) MB.*/idle \1 query \2/'; }
echo "== $1 $(git -C $B log -1 --format=%s | cut -c1-60) (load $(uptime | sed 's/.*averages*: *//'))"
for i in $(seq ${2:-4}); do a=$(cd $B && CARGO_TARGET_DIR=$BT m); h=$(m); echo "rev: $a | head: $h"; done
```

## 2. Linux run of the tray fix

Not yet run on Linux. Plan: one container from `rust:1.99` (arm64), apt packages as in CI (`libwebkit2gtk-4.1-dev
libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev xvfb`), copy of the tree, `cargo test -p debrief-app --lib --
tray::tests:: lifecycle::tests::quitting_finishes_with_live_tail_jobs_and_the_tray` under `xvfb-run -a`, without a
display, and once with `host_available`'s GTK check reverted (must panic in gtk menu.rs). Also
`cargo check -p debrief-app` for Linux is covered by that build.
