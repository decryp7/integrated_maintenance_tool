# Fuzzing and property-based tests

IMT reads untrusted files, so every parser must survive any input: no panic, no hang, no
unbounded memory. Two layers check this.

## Property-based tests (proptest)

Run with the normal test suite; they are fast (a few seconds) and deterministic per seed.

```sh
cargo test -p imt-core --test properties
cargo test -p imt-plugin-registry -p imt-plugin-windows -p imt-plugin-web -p imt-plugin-otel -p imt-plugin-syslog --test properties
```

* Default is 256 cases per property. Run harder with `PROPTEST_CASES=10000 cargo test --release ...`.
* Every case runs on a helper thread with a 15 s limit (`guard`), so a hang fails the test
  instead of blocking CI.
* Inputs are arbitrary bytes/strings, plus mutations (bit flips, inserts, deletes, truncation,
  splices) of `/samples` prefixes, crate fixtures and valid seeds (queries, stack traces,
  timestamps, templates, profile fragments, gzip/zip archives).
* `imt-core/tests/properties.rs` also checks invariants: paging partitions the matching ids with
  no duplicates, the timeline histogram total equals the matching records with timestamps,
  CLEF export read back by the JSON importer preserves ts/level/message/fields, and RFC 3339
  timestamps parse back to the same millisecond.
* On failure proptest prints the minimal failing input and stores it in
  `proptest-regressions/` next to the test; commit that file together with the fix.
  A bug you cannot fix yet gets a test marked `#[ignore = "bug: <description>"]`.

## Fuzzers (cargo-fuzz / libFuzzer)

`fuzz/` is a separate workspace (not part of `cargo build --workspace`) that needs nightly Rust.

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked

cd fuzz
cargo +nightly fuzz list
mkdir -p work/fuzz_json
cargo +nightly fuzz run fuzz_json work/fuzz_json corpus/fuzz_json -- \
    -max_total_time=300 -rss_limit_mb=2048 -max_len=131072 -timeout=20
```

Pass the working directory first and the committed seed corpus second: libFuzzer writes new
inputs only to the first directory, so `fuzz/corpus/` stays small (seeds copied from
`/samples` and the crates' `tests/fixtures`) and `fuzz/work/` is ignored by git.

| Target | What it exercises |
| --- | --- |
| `fuzz_json`, `fuzz_csv`, `fuzz_text` | built-in importers, default profile (decode, sniff, import) |
| `fuzz_timestamp` | `TimestampParser` (parse, parse_time_on, user formats), `to_strftime` |
| `fuzz_profile_toml` | `Profile::from_toml` and `CompiledProfile::compile` |
| `fuzz_ql` | query language `parse`, `completions` (first 2 bytes = cursor), `run` |
| `fuzz_archive` | gzip/zip `expand` and `read_entry` |
| `fuzz_registry` | `.reg` importer |
| `fuzz_evtx` | EVTX binary importer, `read_evtx` |
| `fuzz_w3c_access_har` | W3C/IIS, access log, HAR importers (first byte selects) |
| `fuzz_otel` | OTLP/JSON importer |
| `fuzz_syslog` | syslog and journald importers (first byte selects) |
| `fuzz_perfmon_csv` | PerfMon CSV and `.nfo` importers, counter path parser |
| `fuzz_prof_speedscope`, `fuzz_prof_folded`, `fuzz_prof_cpuprofile`, `fuzz_prof_chrome_trace`, `fuzz_prof_pprof`, `fuzz_prof_perf_script`, `fuzz_prof_nettrace`, `fuzz_prof_nettrace_heap`, `fuzz_prof_heapsnapshot` | profiling importers (`imt-plugin-profiling`, built without the plugin feature), then every view over what they produced (`imt_profile::fuzz_views`) |
| `fuzz_dotnet_ipc` | .NET diagnostics IPC replies (`imt-plugin-dotnet`, without the plugin feature); its ETW events are `imt-etw`'s and fuzzed by `fuzz_native_etw` |
| `fuzz_native_etw` | ETW events of a native capture (kernel, stack walks, context switches, images, heap provider, CLR) through `imt-etw`'s converter and `.etl` scanner, symbols for the collected addresses, then the nettrace importer (ADR 0022) |
| `fuzz_native_pe` | PE headers and the CodeView PDB identity (`imt_symbols::pe`), and the symbol-cache path a PDB name becomes |
| `fuzz_dotnet_etw`, `fuzz_dotnet_ipc` | .NET capture (`imt-plugin-dotnet`, without the plugin feature): ETW event payloads through the NetTrace converter, the `.etl` process scanner and back through the nettrace importer; diagnostics IPC replies |
| `fuzz_image` | The AI's image sanitiser (ADR 0025): any bytes through signature sniffing, the claimed-size guard, the decoders (PNG, JPEG, BMP, GIF, WebP) and the re-encoding, within small limits |
| `fuzz_vision_facts` | What a model writes about a screenshot (strict JSON, bounded) and the EXIF capture-time reader |

CI (`.gitea/workflows/fuzz.yml`) runs every target for 5 minutes each night (and on demand
through `workflow_dispatch`, with a configurable duration) and uploads crash inputs as the
`crashes-<target>` artifact. Five shards build the fuzz workspace once
and then fuzz their targets one after another, on the dated nightly in `FUZZ_TOOLCHAIN`
(use the same one locally, e.g. `cargo +nightly-2026-10-01 fuzz run ...`, to reproduce CI).

### Adding a target

1. Create `fuzz/fuzz_targets/fuzz_<name>.rs` (see an existing one; helpers live in
   `fuzz/common/mod.rs`) and add a matching `[[bin]]` entry to `fuzz/Cargo.toml`.
2. Copy a few small valid inputs (a few KB each) into `fuzz/corpus/fuzz_<name>/`.
3. Add the name to a shard's `targets` in `.gitea/workflows/fuzz.yml` and to the table above. A
   shard has one crash-upload step per target (seven); when every shard is full, add a shard.
4. Add a proptest property for the same entry point next to the other ones.

### Triaging a crash

1. Download the artifact (or take `fuzz/artifacts/<target>/crash-*`, `oom-*`, `timeout-*`).
2. Reproduce: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>`
   (a backtrace is printed; `RUST_BACKTRACE=1` helps).
3. Minimise: `cargo +nightly fuzz tmin <target> <crash-file>`.
4. Write a regression test first (AGENTS.md: every bug fix starts with a failing test) that feeds
   the minimised bytes to the same function, then fix the parser, not the fuzz target.
   Keep the minimised input under `fuzz/artifacts-found/` while the bug is open, and add it to
   `fuzz/corpus/<target>/` once fixed so it stays covered.
5. Timeouts and OOMs count as bugs: parsers must enforce limits (`MAX_LINE_LEN`,
   `user_regex::compile`, archive size caps).

Coverage of a corpus: `cargo +nightly fuzz coverage <target> work/<target>`.
