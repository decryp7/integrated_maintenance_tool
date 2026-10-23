# 4. Stable plugin ABI: WIT for WebAssembly, protobuf for processes

- Status: superseded by [ADR 0017](0017-one-ipc-out-of-process-plugins.md) (2026-10-20); amended 2026-10-18 (the WebAssembly part is dropped, see below)
- Date: 2026-10-17

> **Superseded (2026-10-20):** plugins are Rust only and out of process, over Debrief's one frame codec
> (postcard frames on a private local stream) and the `debrief-plugin-api` crate (ADR 0017). The protobuf contract, the
> gRPC transport, the C#/.NET SDK and the conformance kit are deleted.
>
> **Amendment (2026-10-18):** the WebAssembly component contract (WIT `debrief:plugin@1`) is dropped.
> Third-party plugins use the process contract only: `debrief.plugin.v1` over local gRPC with Arrow
> batches, with the Rust and C#/.NET SDKs and `debrief-plugin-check`. The rest of this record stands.

## Context

Built-in plugins are Rust crates compiled into Debrief and use Rust traits directly. Third parties
(and plugins in other languages) need a contract that stays stable across Debrief releases and Rust
compiler versions. Rust has no stable ABI: loading a Rust `cdylib`/`dylib` that exchanges trait
objects is undefined behaviour across compiler or dependency versions, and native code loaded
in-process can crash Debrief (ADR 0002).

Options considered:

1. **Compile-time only** (today): no ABI problem, but every plugin needs a Debrief rebuild.
2. **Native dynamic libraries with a C ABI** (`extern "C"`, `abi_stable`, `stabby`): fast, but
   in-process native code defeats crash isolation, needs per-OS/arch builds and careful
   memory-ownership rules.
3. **WebAssembly components with WIT interfaces** (wasmtime): a versioned, language-neutral
   interface definition; sandboxed (memory limits, epoch interruption, no ambient I/O); one
   binary for every OS.
4. **Out-of-process plugins over gRPC** (ADR 0002): the contract is the `.proto` schema; any
   language; full crash isolation.

## Decision

Debrief supports **no native dynamic-library plugins**. The stable plugin ABI is two contracts that
describe the same capabilities:

- **WIT package `debrief:plugin@1.x`** for WebAssembly components (preferred for third-party
  importers, transforms, marker kinds, annotators, exporters and settings).
- **Protobuf package `debrief.plugin.v1`** for plugin processes (for code that needs native
  libraries, OS APIs or another runtime, e.g. .NET TraceEvent, profilers).

Rules:

- **One capability model.** Both contracts expose the same contribution kinds as the Rust
  traits (importer with sniff/import/import-tail, transform, marker kinds, annotator, exporter,
  settings section, services, events). Host-side adapters map each contract onto the Rust
  traits, so the core, the guard, quarantine, settings and the UI see no difference.
- **Data in Arrow.** Record batches cross the boundary as Arrow IPC bytes in both contracts;
  small structured values (settings, manifests, markers) use the contract's own types.
- **Versioning.** Semver on the package: minor versions only add (new functions, optional
  fields, new enum cases that hosts and plugins must tolerate); a breaking change is a new major
  (`debrief:plugin@2`, `debrief.plugin.v2`), and the host supports the previous major for at least one
  release. The handshake negotiates the version and the capabilities a plugin implements;
  missing capabilities are simply absent.
- **Manifest.** Every external plugin ships a manifest (id, version, contract and version,
  requires/provides per ADR 0003, requested permissions such as "read files the user opened"
  or "network to example.com"). Permissions are shown at install and enforced by the host
  (WASI capabilities for components; the gRPC host only proxies what is granted).
- **Conformance.** A plugin test kit (SDK crate plus a CLI) runs contract tests against a
  plugin component or process before it can be installed; the same tests run against the
  built-in adapters in CI.
- **Built-in plugins** keep using Rust traits (no serialisation cost) but must stay expressible
  through the contracts, so they could be moved out without changing behaviour.

### Rollout order (2026-10-17)

The gRPC contract is the **primary, stable** path for third-party plugins now: gRPC/protobuf and
their toolchains are mature in every language, including C#. The WebAssembly contract is added
later as a **preview** and becomes stable only after **WASI 1.0** ships. As of October 2026, core
WebAssembly is stable, WASI 0.2 (2024) and 0.3 (ratified June 2026) are stable snapshots, but WASI
1.0 has not been released and language tooling is uneven (e.g. C# components are still preview).
Until then a WASM plugin may need rebuilding when Debrief moves to a newer WASI version.

### First-party code is Rust only (2026-10-17)

Everything Debrief builds itself is Rust: the engine, shell, built-in plugins, workers (AI, plugin
host) and profiling components, including the .NET CLR profiler agent (a Rust `cdylib` COM
server via the `windows` crate) and ETW/`.etl` decoding (Windows TDH/ETW APIs from Rust). No
first-party C#, C++ or other runtimes ship with Debrief. Plugins in other languages are supported
through the protobuf contract (and WIT later): any language with gRPC can implement
`debrief.plugin.v1` and be checked with the conformance kit. Debrief ships official Rust and C#/.NET SDKs
(the latter with a template and sample plugins, `sdk/dotnet`); SDKs for further languages are
built when a real plugin needs one. SDKs are developer
tooling, not part of the app.

## Consequences

- Third-party plugins work across Debrief versions without recompiling Debrief, in any language that
  targets WebAssembly components or speaks gRPC.
- A buggy external plugin cannot corrupt Debrief's memory.
- Cost: two schemas to keep aligned (mitigated by one capability model and shared contract
  tests), serialisation at the boundary (mitigated by Arrow batches), and wasmtime in the app.
