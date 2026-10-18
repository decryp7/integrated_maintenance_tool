# 9. Inference runs the official prebuilt llama-server, downloaded on demand

- Status: accepted
- Date: 2026-10-17

## Context

The AI worker (`imt-ai-worker`, ADR 0002) linked llama.cpp through the `llama-cpp-2` crate. Building
it needed CMake, libclang and a C/C++ toolchain, the Vulkan SDK for the GPU build, and several
minutes per target; every IMT build and every CI runner paid for it, and each GPU backend (Metal,
Vulkan, CUDA, ROCm, SYCL) was a separate compile we would have to maintain. llama.cpp already
publishes tested prebuilt binaries for every platform and backend with each release, including
`llama-server`, an HTTP server with a stable completion API. Bundling all of them in the installer
would add from 50 MB (CPU + Vulkan) to well over 1 GB (CUDA runtimes) per platform, most of which a
given machine never uses.

## Decision

- **The worker runs llama-server as its own child.** The worker process and its gRPC contract
  (`proto/llm.proto`) are unchanged; a new `WorkerBackend` (`imt-ai-worker/src/server.rs`) starts
  `llama-server -m <model> -c <ctx> -ngl <layers> --threads <n> --parallel 1 --fit off --no-webui
  --no-slots --offline --log-verbosity 1` on 127.0.0.1 and a free port, and talks to it with a
  small hand-written HTTP/1.1 client whose reads poll so a cancel closes the connection at once
  (`/health`, `/tokenize` for the context check, `/props` for the real context, `/completion`
  streamed as SSE with `cache_prompt`, `json_schema`, greedy or top-k 40 / top-p 0.95 / seed 1234,
  and `preserved_tokens` so `<tool_call>`/`<think>` tags stay in the text even when they are special
  tokens). Stop types map to `Stop`/`Length`; token counts come from the server.
- **Hardened child.** An empty environment plus a short allow-list (llama-server reads `LLAMA_ARG_*`
  variables, which could otherwise enable its agent tools or the web UI); a random API key passed in
  the environment, never on the command line; no window, below-normal priority; tied to the worker's
  lifetime (Windows job object with kill-on-close, Linux `PR_SET_PDEATHSIG`, every Unix an `atexit`
  hook and kill on drop, which is what runs when IMT closes the worker's stdin). A server that dies is
  reported with its last error lines and started again on the next request or load. Errors keep the
  worker protocol's typed codes (`CONTEXT_OVERFLOW` with the same data, `INVALID_PARAMS` for a bad
  schema, `LOAD_FAILED` with an actionable message; `data.reason = "no_runtime"` when nothing is
  installed).
- **Builds are downloaded like models, not bundled.** A pinned catalog
  (`crates/imt-ai/src/runtimes/llama-runtimes.json`: release, build tag, and for every platform and
  build the asset URL, size and SHA-256) is embedded in IMT and regenerated with
  `scripts/update-llama-runtimes.py`. The runtime manager (`imt_ai::runtimes`) downloads a build with
  the model downloader (free-space check, resume, retries, SHA-256 before use) from GitHub only (and
  its release CDN), unpacks it defensively (only `llama-server`, shared libraries and licences; flat
  names, no `..`, links only to siblings, caps on entries and sizes) into
  `<data>/ai/runtimes/<tag>/.staging-<id>` and renames it to `<tag>/<id>` only when complete. The user
  can install several, see which work, switch and delete them.
- **Choosing a build.** The setting *Inference build* is `auto` or an id; *auto* takes the first
  installed build that works in the order Metal, CUDA 13, CUDA 12, Vulkan, CPU. ROCm, SYCL and
  OpenVINO are offered but only used when chosen. "Works" means `llama-server --list-devices` runs and,
  for a GPU build, lists a real GPU (software Vulkan devices such as llvmpipe do not count). When the
  chosen build cannot be used or does not start, the worker falls back to the next one and says so in
  the backend name ("CPU (fallback: the Vulkan build did not start (...))"). GPU layers are still planned
  by `recommend::plan_offload` from the free memory the build reports. The set-up suggests a build from
  the hardware probe (CUDA for NVIDIA, Vulkan for other real GPUs, Metal on Apple Silicon, else CPU).
  A custom llama-server path (Settings → AI, advanced) replaces the downloaded builds.
- `imt-ai-worker --list-builds` prints which builds are installed and usable (JSON,
  `imt_ai::runtimes::BuildsReport`), so the probing of native code stays out of IMT's process.

## Consequences

- Building IMT needs only Rust and Node; the `llama-cpp-2` dependency and the worker's `metal`/`vulkan`
  features are gone, and CI no longer compiles llama.cpp or needs llama binaries. The fake llama-server
  of the tests covers the backend; a real end-to-end test downloads the pinned CPU build and a tiny model
  when `IMT_LLAMA_E2E=1`.
- **Download sizes** (pinned b11669 = v0.7.0, compressed): Windows x64 CPU 20 MB, Vulkan 32 MB,
  CUDA 12 658 MB and CUDA 13 578 MB (each with its cudart runtime), ROCm 258 MB, SYCL 149 MB, OpenVINO
  99 MB; Linux x64 CPU 18 MB, Vulkan 30 MB, CUDA 12 767 MB, CUDA 13 594 MB, ROCm 245 MB, SYCL 55 MB,
  OpenVINO 117 MB; macOS arm64 (Metal) 12 MB; macOS x64 (CPU) 12 MB. Unpacked, without the other
  llama.cpp tools, the Windows CPU build is about 43 MB. Links in the Unix archives become copies,
  which costs a few MB per build. The installer itself grows by nothing.
- **API stability.** `llama-server`'s HTTP API is not a versioned contract (the v0.7.0 release changed
  the default port and the chat layer). IMT pins one release, passes every option explicitly and uses
  only `/health`, `/tokenize`, `/props` and `/completion`; updating the pin is a reviewed change that
  must pass the end-to-end test. Builds of older pins on disk keep working with the worker and are
  listed as outdated so the user can delete them.
- **Supply chain.** Only assets named in the checked-in catalog are fetched, from `github.com` and
  GitHub's release CDN over https, and each is checked against its pinned SHA-256 before it is
  unpacked or run; a mismatch deletes the file. The checksums come from the GitHub release API and
  `update-llama-runtimes.py --verify` recomputes them by downloading; a new pin shows up as a diff of
  URLs and hashes. We trust ggml-org's release pipeline (their releases also carry build attestations).
- **macOS.** IMT downloads with its own HTTP client, so files do not normally get the quarantine
  attribute; any that is present is removed only after the checksum check. Apple Silicon runs only
  signed code: the official binaries are ad-hoc (linker) signed, and when `codesign --verify` rejects
  a file IMT ad-hoc signs it (`codesign --force --sign -`). llama-server is a separate process, so the
  notarised IMT app's hardened runtime and library validation do not apply to it and no entitlement is
  needed; Gatekeeper does not assess a non-quarantined executable started by another process. A
  managed Mac whose policy forbids unsigned or ad-hoc-signed code needs the administrator to allow it,
  or a custom llama-server path to a build they trust.
- The worker's own memory no longer includes the model; `LoadResult.rss_bytes` adds llama-server's
  resident memory, and IMT's status should count the worker's children too.
- Licences: llama.cpp is MIT; the CUDA runtime archives carry NVIDIA's redistributable licence and are
  downloaded by the user from llama.cpp's release, not redistributed by IMT.
