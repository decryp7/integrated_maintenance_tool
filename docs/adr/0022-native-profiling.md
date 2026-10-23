# 22. Native (C/C++) profiling: ETW and suspend-and-sample captures as NetTrace, one symbol library

- Status: accepted (2026-10-21)
- Related: [ADR 0019](0019-dotnet-profiling.md) (.NET profiling: the template), [ADR 0017](0017-one-ipc-out-of-process-plugins.md)
  (out-of-process plugins, elevated plugin processes), ADR 0016 (jobs, tool polling, typed `denied` errors),
  [docs/profiling.md](../profiling.md), AGENTS.md

## Context

Debrief can record .NET processes (ADR 0019) but not native C/C++ services and desktop applications, which are the other
half of the Windows estates its users maintain. They want to know where a native process burns CPU, where its threads
wait (locks, I/O, sleeps), which call stacks grow its heap, and to open `.etl` traces recorded by WPR/xperf in the same
views. Many of these processes run in production: nothing may be loaded into them, and recording must work without
administrator rights when the user has none (with less detail) and with them (full detail).

## Decision

### 1. Safety boundary: events and debugger-style sampling only

Captures use only what the OS publishes (ETW) or what a debugger may read from outside the process (thread contexts and
memory, read while the thread is suspended). Debrief **never** injects code, DLLs, threads, hooks or APCs into the target:
no `CreateRemoteThread`, `WriteProcessMemory`, `SetWindowsHookEx`, `QueueUserAPC`, profiler-API attach or IAT/inline
hooking. Anything that can only be learned by injection (exact call counts, instrumentation, allocations of a process that
is not traced by the heap provider) is out of scope; a future need requires a new ADR.

### 2. One built-in plugin, two capture modes

The plugin crate `crates/debrief-plugin-native` (`net.decryptology.native`, served by `debrief-plugins` on `debrief-plugin-api`)
contributes:

| Tool | Process | Operations |
|---|---|---|
| `native-profiler` | normal | `defaults`, `processes`, `start` (suspend-and-sample), `status` (pollable), `stop`, `etl-processes`, `etl-convert` |
| `native-profiler.etw` | **elevated** (ADR 0017 §7) | `start`, `status` (pollable), `stop` |

and a profile importer for `.etl` files. Two tools rather than one with mixed operations because a task is cancelled
(`Tool::stop`, the Jobs pane) in the process that owns it, and the host routes a tool's `stop` to its elevated process
only when every operation of the tool is elevated.

- **ETW (administrator).** One real-time system-logger session per capture (`Debrief-native-<debrief pid>-<target>-<n>`):
  kernel `PROFILE` (1 ms) with stack walks, `CSWITCH` + `DISPATCHER` (ready thread) with stack walks for waits,
  process/thread/image-load events; optionally the **heap provider** (`{222962AB-6180-4B88-A825-346B75F2A24A}`,
  PID-filtered, with stacks) for allocations, and the CLR runtime and rundown providers when the target has loaded
  the CLR (mixed stacks, §5). It is the plugin's first elevated operation: the host starts the plugin's elevated
  process through the administrator prompt on first use; declining is a typed `denied` failure whose hint is
  "Restart as administrator". x86 (WOW64) and x64 targets are recorded from the 64-bit Debrief (kernel stack walks keep
  the 32-bit user frames; pointer size from the event header).
- **Suspend-and-sample (no administrator).** A sampler thread in the normal plugin process opens the target with
  `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION` and each thread with `THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT |
  THREAD_QUERY_INFORMATION`; per tick it suspends one thread, reads its context (`GetThreadContext`, or
  `Wow64GetThreadContext` for a 32-bit target), **resumes it** and walks the stack with DbgHelp `StackWalkEx` over
  `ReadProcessMemory` (the stack is copied while the thread is suspended, so the walk itself does not hold it).
  The resume is an RAII guard: it runs on every return path, on error and on unwinding. Rate: 100 Hz by default,
  capped at 1 kHz, one thread suspended at a time for at most the copy of its context and stack (bounded to 256 KiB).
  A thread whose CPU time (`GetThreadTimes`) advanced since its previous sample counts as running (a CPU sample); one
  that did not counts as waiting (a `ThreadWait` of one interval at its stack, the wait reason derived from the leaf
  frame: delay, lock, object, I/O completion). Wall-clock sampling therefore gives CPU and lock/wait hot spots
  without rights the user does not have.

The plugin's manifest asks for `Permission::WriteFolders` (captures and the symbol cache in Debrief's data folder) and
`Permission::Network` (symbol downloads, used only when enabled). `WriteFolders` also keeps the normal process at
normal integrity: a low-integrity process cannot open the user's processes for reading. A dedicated "inspect
processes" permission would describe this better (redesign note).

Both modes run captures as tool tasks of kind `capture` (Jobs of kind Capture, several at once), are stopped with
`stop` (kept) or cancelled from the Jobs pane (file deleted), have a duration cap (1 h) and a size cap (1 GiB), and
stop by themselves when the target exits.

### 3. One file format: NetTrace with Debrief's extension events

Every capture is written as NetTrace through the **shared streaming writer** and converter in `crates/debrief-etw`
(§6), so the existing nettrace importer, views and findings open it. Kernel samples become
`SampleProfiler/ThreadSample`, waits `Debrief-Windows-Kernel/ThreadWait`, images `ImageLoad` (ADR 0019). New
`Debrief-Windows-Kernel` events (documented in `debrief_plugin_profiling::nettrace::ext`):

| Event | Id | Payload |
|---|---|---|
| `NativeSymbol` | 4 | address `u64`, function start `u64`, line `u32`, function name, source file (UTF-16, NUL-terminated) |
| `HeapAlloc` | 5 | heap `u64`, address `u64`, size `u64` (stack: the allocating stack) |
| `HeapFree` | 6 | heap `u64`, address `u64` |
| `HeapReAlloc` | 7 | heap `u64`, new address `u64`, old address `u64`, new size `u64` (stack: the reallocating stack) |

The importer resolves addresses in this order: CLR methods (managed frames), `NativeSymbol`s (native frames named
`function`, module from the image, the function's source file and first line), images (`module+0xoffset`), `0x…`.
Heap events pair by address: allocations become `allocBytes`/`allocCount` samples at their stacks, and allocations
not freed by the end of the capture become **`liveBytes`** samples (a new inclusive metric, "Unfreed bytes": the heap
growth by stack). The table of outstanding allocations is bounded (`Limits::max_samples`); beyond it the importer
notes the cut.

### 4. Symbols: `crates/debrief-symbols`, a library shared with dumps (batch 8)

Symbolisation runs once per capture, after recording, in the process that recorded it: the converter collects the
distinct user-mode addresses (bounded) and the images, `debrief-symbols` resolves them, and the `NativeSymbol` events are
appended before the stream ends. The library's API (stable; the dumps/UMDH work uses it):

```rust
// crate root
pub struct SymbolConfig { pub paths: Vec<PathBuf>, pub cache: Option<PathBuf>, pub server: Option<ServerConfig> }
pub struct ServerConfig { pub url: String, pub timeout: Duration, pub max_bytes: u64 }   // ::microsoft(), .check()
pub fn parse_search_path(text: &str) -> Result<Vec<PathBuf>, SymbolError>;            // "C:;D:": absolute folders
pub struct Module { pub path: PathBuf, pub base: u64, pub size: u64 }                  // .name(), .contains(addr)
pub struct Symbol { pub function: String, pub start: u64, pub module: String, pub file: Option<String>, pub line: Option<u32> }
pub enum ModuleSymbols { Pdb, Exports, None { why: String } }
pub trait Resolver: Send {
    fn add_module(&mut self, module: &Module) -> ModuleSymbols;
    fn resolve(&mut self, address: u64) -> Option<Symbol>;
}
pub fn resolver(config: &SymbolConfig) -> Result<Box<dyn Resolver>, SymbolError>;     // DbgHelp; Unavailable off Windows
pub struct Resolved { pub symbols: HashMap<u64, Symbol>, pub modules: Vec<(String, ModuleSymbols)>, pub unresolved: usize }
pub fn resolve_all(r: &mut dyn Resolver, modules: &[Module], addresses: &[u64], max: usize) -> Resolved;
pub fn with_dbghelp<R>(f: impl FnOnce() -> R) -> R;     // DbgHelp is single-threaded: one lock per process
pub enum SymbolError { Unavailable, InvalidPath(String), DbgHelp { call, message } }

pub mod pe {      // pure, fuzzed
    pub trait ImageSource { fn read_at(&mut self, offset: u64, out: &mut [u8]) -> usize; }   // &[u8], File
    pub fn parse(src: &mut dyn ImageSource) -> Result<PeInfo, PeError>;
    pub fn parse_file(path: &Path) -> Result<PeInfo, PeError>;
    pub struct PeInfo { pub machine: Machine, pub image_size: u32, pub timestamp: u32, pub image_base: u64, pub pdb: Option<PdbId> }
    pub struct PdbId { pub guid: [u8; 16], pub age: u32, pub path: String }               // .key(), .file_name()
}
pub mod config { pub fn cache_file(cache: &Path, id: &PdbId) -> Option<PathBuf>; pub fn safe_pdb_name(id: &PdbId) -> Option<&str>; }
pub mod fetch {   // feature "server" (default)
    pub trait Fetcher: Send + Sync { fn fetch(&self, id: &PdbId, cache: &Path) -> Result<PathBuf, FetchError>; }
    pub struct HttpFetcher;                                                              // ::new(ServerConfig)
}
#[cfg(windows)] pub mod walk {   // StackWalkEx over a memory source
    pub trait Memory { fn read(&self, address: u64, out: &mut [u8]) -> usize; }
    pub struct ProcessMemory(pub HANDLE);                                               // ReadProcessMemory
    pub struct Snapshot<'a> { pub base: u64, pub bytes: Vec<u8>, pub rest: &'a dyn Memory }
    pub enum Context { Amd64(Box<CONTEXT>), X86(Box<WOW64_CONTEXT>) }                   // .stack_pointer(), .instruction_pointer()
    pub struct Walker;   // ::live(process) | ::offline(); add_module(&Module) -> bool; walk(&Context, &dyn Memory, max) -> Vec<u64>
}
```

- The symbol search path is configurable (plugin settings: *Symbol path*; the dialog prefills it); the cache is
  `<app data>/symbols` in symstore layout (`<pdb>/<GUID><age>/<pdb>`).
- **The Microsoft symbol server is off by default.** When the user enables it (*Settings ▸ Native profiling ▸ Download
  symbols from Microsoft*), missing PDBs of system modules are downloaded over HTTPS (certificate checking always on,
  platform verifier) into the cache, size- and time-bounded, at most one download per PDB per capture; the plugin
  process has network access only because its manifest asks for `Permission::Network` (the Plugins settings show it).
  The app has no shared download stack for plugins yet (ADR 0017 §8, `SafeHttp`, deferred), so the AI *Download proxy*
  setting does not apply to symbol downloads; that is a known gap.
- Modules without a PDB resolve through exports (DbgHelp's default) and are flagged; addresses no module covers stay
  `module+0xoffset` or `0x…`, and the profile notes say how many and why.

### 5. Mixed .NET and native stacks (best effort)

When the target has loaded the CLR, the ETW capture also enables the CLR runtime provider (loader, JIT) and, at stop,
the rundown provider, as ADR 0019 does. Kernel stack walks contain both native and JIT-compiled frames; the importer
resolves JIT addresses from the method load/rundown events and native ones from the symbols, so one stack shows both.
Limits: suspend-and-sample walks stop at JIT frames whose unwind data the runtime registered only in-process (x64
dynamic function tables need the runtime's out-of-process callback, which would mean loading runtime code into Debrief's
sampler; not done), so managed parts of sampled stacks are truncated; frames of methods JIT-compiled and unloaded
before the rundown show as addresses.

### 6. Shared ETW code: `crates/debrief-etw`

The ETW consumer glue, the kernel and heap payload decoders, the `.etl` scanner, the ETW → NetTrace converter and the
NetTrace writer move out of `debrief-plugin-dotnet` into a library crate, generalised (session configuration by flags and
providers, converter options for heap events and symbol collection). Both profilers use it: `debrief-plugin-dotnet` keeps
only its recording control (presets, CLR providers, rundown and heap-walk milestones) on `debrief_etw::session`, and its
copies of the decoders, converter, writer and `.etl` reader are deleted.

### 7. Elevated output

The elevated ETW collector never writes the capture itself. It takes no path argument and streams the NetTrace bytes
to the host through `ToolCtx::outputs()` (ADR 0017 §7, amendment of 2026-10-22; protocol 1.1); the unelevated host
checks the folder and file names, writes `<data>/captures/native/<name>` with a hidden partial file, and renames it
into place when the stream ended as announced. A failed or cancelled capture is dropped without completing its
stream, so nothing is left. The stream's limit is the capture's size limit plus a 64 MiB allowance for the closing
blocks (the address symbols) the recorder writes after it reaches the limit. The capture is reported as a tool output
at the path the host saved it, so only the calling window may open it. What the elevated process still writes is its
symbol cache (only when the user enabled a symbol server): the host's data folder is accepted for that only when it
lies under the elevated token's `%APPDATA%` or `%LOCALAPPDATA%`.

### 8. Findings

Native findings are engine findings in `debrief-profile` (ADR 0019's pattern): **hot native function** (self time in a
native frame with a symbol, with its module and source file), **wait hot spot** (the native function in which threads
waited longest, by `waitNs` at the first non-system frame above the wait), and **heap growth by stack** (the call
path holding most `liveBytes`). Each links to its evidence view.

### 9. `.etl` import

There is **one `.etl` importer** (id `etl`), this plugin's, for .NET and native processes alike: the converter keeps
CLR events (managed frames) as well as kernel, heap and image events, and names native frames from symbols, so it is a
superset of the .NET plugin's former importer, which is deleted together with the `dotnet-profiler` tool's
`etl-processes` / `etl-convert` operations. Dropping an `.etl` file opens its busiest .NET process (else its busiest
process; notes name the others); *File ▸ Open process from .etl…* (the native tool's `etl-processes` /
`etl-convert`, processes listed .NET first) opens another. Windows only; elsewhere "ETL import needs Windows".

## Consequences

- Native processes can be profiled without rights (sampling, wall-clock, coarser) or with them (ETW, every CPU sample,
  waits with their readying thread, heap stacks), with the same views and findings as .NET captures.
- Suspend-and-sample briefly stops one target thread at a time (microseconds per sample); its overhead is bounded by
  the rate cap and documented. ETW heap tracing slows allocation-heavy targets noticeably while it runs.
- Real ETW recordings need an elevated machine: they are `#[ignore]` tests behind `DEBRIEF_ETW_TESTS=1`. Suspend-and-sample
  is tested for real in CI against a Rust fixture (`tests/fixtures/native-busy`), with symbols from its PDB.
- Two copies of the ETW code exist until `debrief-plugin-dotnet` moves onto `debrief-etw`.
