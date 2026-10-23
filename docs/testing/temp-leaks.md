# Temp folders left by tests

Tests must not leave `debrief-*` or `imt-*` folders in the OS temp directory. CI fails the `build` workflow's
`rust tests` and `frontend` jobs when a run leaves one.

- **Check**: `scripts/ci/check-temp-leaks.mjs`. `--snapshot <file>` records the matching folders that exist before the
  tests; `--check <file>` exits 1 naming the ones that appeared since. Without a baseline every matching folder counts.
  `--dir <path>` scans another folder. Its test is `scripts/ci/check-temp-leaks.test.mjs`.
- **Rust tests**: make scratch folders with `tempfile::TempDir` (or `tempfile::Builder::new().prefix("debrief-...-")`) and
  keep the guard alive for the test, so the folder is removed on panic too. Unit tests in `debrief-core` use
  `crate::test_dirs::scratch_dir(prefix)`, which keeps the guard in a thread-local for the test's thread.
  Never build a path from `std::env::temp_dir()` and `remove_dir_all` it at the end of the test: a failing assert skips the cleanup.
- **Measure locally**: run the tests with `TMPDIR` pointing at an empty folder and list it afterwards:
  `mkdir /tmp/t && TMPDIR=/tmp/t cargo test -p debrief-core; ls /tmp/t`.
