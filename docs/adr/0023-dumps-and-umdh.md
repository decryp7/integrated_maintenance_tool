# 23. Crash/memory dumps and UMDH heap logs: a sandboxed reader plugin, an engine crate, problem-first views

- Status: accepted (2026-10-21)
- Related: ADR 0017 (one IPC, out-of-process plugins, sandbox, elevation, typed path arguments), ADR 0016
  (command table, `CmdError`, path grants, tool outputs), ADR 0019 (.NET profiling: problem-first findings and
  "Explain with AI"), ADR 0014 (engine as the source of truth)

## Context

Support engineers get crash dumps (`.dmp`, `.mdmp`: minidumps, minidumps with heap, full dumps from WER,
ProcDump, Task Manager, Visual Studio) and heap-growth logs from Windows' User-Mode Dump Heap tool (UMDH) and
today need WinDbg or Visual Studio to read them. IMT already listed a minidump as a few log records (the Windows
plugin's `minidump` importer: system, exception, a stack *scan* of the faulting thread, modules), but could not
show threads, walk stacks, name frames, see deadlocks or compare heap snapshots. Users also want to write a dump of
a running process without installing ProcDump.

Dumps are untrusted, often huge (a full dump is the process' whole memory: gigabytes) binary files; UMDH logs are
large text files whose format is documented only by the tool's own output. Both must be parsed without risk to
IMT, and the answers must lead with the problem (what crashed and where; what leaks), like ADR 0019's findings.

## Decision

1. **Two crates, the profiling split.** `imt-dump` is the engine: the document model (`Dump`, `UmdhLog`), its wire
   form from the plugin process (postcard, checked against `Limits` on decode, like `imt_profile::wire`), the
   per-window store (`DumpDocs`) and every analysis (overview, exception card, threads with their state, stacks,
   modules, findings, UMDH growth, stack tree, timeline markers, the AI summary). It is pure (no I/O) and is what
   the app links. `imt-plugin-dumps` holds the readers and the capture tool; the app never links it (ADR 0017).
2. **Untrusted parsing out of process, least privilege, two plugins in one crate.**
   - `net.decryptology.dumps` (readers): locked sandbox (no network, no writes, low integrity on Windows,
     3 GiB memory cap). Its only extra right is the new permission **`ReadFolders`** ("read files in folders you
     configure, never write them"): its symbol folders. On Windows low integrity already reads and cannot write;
     elsewhere it becomes a read-only file system (`FsAccess::Paths { read: ["/"] }`), since the folders are
     settings that change after the process starts.
   - `net.decryptology.dumps.capture` (the `dumps.capture` tool): `WriteFolders` (normal integrity, needed to open
     another process for reading and to write into the folder the user chose), 256 MiB.
3. **A document contribution, files lent by ranges.** The plugin contract gains `DocumentImporter`
   (`SniffDocument`, `ImportDocument` → encoded pieces + `DocumentDone`; `DocumentImporterCap { id, produces }` in
   the manifest; variants added at the end of the enums, ADR 0017 §1). Unlike `ProfileImporter`, the importer reads
   the file through `RangeRead` (the host answers `ReadFile` from the open file, `Lent::ranges`), so a full dump is
   never copied whole into either process: the reader takes the minidump's prefix (everything with 32-bit offsets:
   directory, threads, contexts, modules, names, handles, memory info) and only the stack memory of each thread from
   the full-memory list, and rewrites the `Memory64List` to describe what it kept. Other dumps are read whole up to
   1 GiB. `imt-plugin-host::document` registers a proxy per importer on `imt_dump::IMPORTERS_POINT`; the app's
   store reaches them through guarded services like profile importers. Profiles could move to this contribution
   later (rule of three: profiles, dumps, UMDH); not in this change.
4. **Minidumps with rust-minidump.** `minidump` parses the streams (system info, misc info, exception, thread list
   and names, modules with CodeView ids, unloaded modules, handle data, memory info list), `minidump-unwind` walks
   every thread (x86 and x64; ARM as the crate supports): call frame information when symbols provide it, frame
   pointers, stack scanning; each frame keeps how it was found (`context`, `cfi`, `framePointer`, `scan`). Bounds:
   4,096 threads, 256 frames per thread, 256 KiB of stack above the stack pointer per walk, 1 MiB of stack kept per
   thread of a full dump (256 MiB in all), 4,096 modules, 64 handle types, a 60 s budget for all walks (later threads
   keep their context frame, noted). Damaged optional streams become notes; only a missing header or directory
   fails the import. `minidump-processor` is not used (async, and its symbolisation and output model duplicate
   what the engine does).
5. **Symbols behind a port.** `imt_dump::symbols::SymbolProvider` (`lookup(module, rva) -> Symbol`) is the seam.
   This change implements it in pure Rust with the `pdb` crate (procedures with lengths from the module streams,
   public symbols for the rest, simple MSVC names undecorated), over **local symbol folders** in the dumps plugin's
   settings (symbol-store layout `<name>.pdb/<GUID><age>/<name>.pdb` and flat), refusing a PDB of another build when
   the module records its id. Nothing is downloaded. `imt-symbols` (ADR 0022: DbgHelp, the configurable symbol
   path, the opt-in Microsoft symbol server) is not wired in yet: its `Resolver::add_module` takes the module's image
   path on disk, which a dump from another machine does not have, and a symbol-server download needs the network,
   which the readers' sandbox denies. The adapter needs an identity-based load (PDB name, GUID, age, as in the
   module's CodeView record) in `imt-symbols` and either the `Network` permission or the download done by the host;
   then it implements `imt_dump::symbols::SymbolProvider` and the readers and the engine do not change. Frames keep the return address debuggers show; lookups use it as is.
6. **UMDH.** The reader recognises snapshots (`umdh -p -f`: `// Loaded modules:`, then `<size> bytes + <overhead>
   at <address> by BackTrace<id>` per allocation with the stack the first time an id appears) and diffs (`umdh old
   new`: `± delta (new - old) count allocs BackTrace<id>`, the count line, the stack, `Total increase == …`), in
   UTF-8 or UTF-16 with a byte-order mark (PowerShell redirection). Numbers are hexadecimal unless the total line only
   adds up in decimal (`umdh -d`). Raw addresses are placed in the listed modules (`module+offset`) and named from the
   symbol folders when possible. **The engine diffs snapshots**, matching stacks by their frames
   (`module!function` per frame), not by back trace id (stable only within one stack database), and groups growth
   by stack, by module and by function of the *culprit frame* (the first frame that is neither an allocator nor in a
   system or runtime module).
7. **Problem-first findings, computed in the engine.** Dumps: the crash cause (exception code decoded with its name
   and meaning, access kind and address, null-pointer and execute hints, fast-fail codes), the likely culprit (the
   first user frame on the faulting thread, preferring walked over scanned frames), a faulting address outside every
   module (with unloaded modules), recursion in a stack overflow, heap corruption (with the page-heap next step),
   threads blocked in lock waits grouped by the user frame that takes the lock (two or more places, or
   `POSSIBLE_DEADLOCK`, is a deadlock hint), a dump without an exception (hang dump), the .NET runtime, handle
   counts, an exhausted 32-bit address space, missing symbols. UMDH: the heap's total change, the top growing
   stacks (≥ 1 MiB or ≥ 10% of the growth; critical at ≥ 50%), stacks that grow in every step of three or more
   snapshots, single allocations ≥ 16 MiB. Waits are recognised by function name, so they need symbols for the
   system modules; the missing-symbols finding says so.
8. **.NET managed state: detection now, the DAC later.** The reader detects the runtime (`clr.dll`/`mscorwks.dll`
   → .NET Framework, `coreclr.dll`/`libcoreclr.so` → .NET) and its version; findings say that managed frames show as
   code without names and that managed stacks and exception objects need the runtime's data access component
   (`mscordaccore`/`mscordacwks`) of exactly that version. Loading it means loading a Microsoft DLL into the
   (sandboxed) reader process and implementing the `ICLRDataTarget` side; it is a follow-up behind a setting, with the
   DAC taken only from the matching locally installed runtime, never downloaded silently.
9. **Capture through the documented API only.** `dumps.capture` `create { pid, kind: mini | heap | full, folder }`
   opens the process with `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ` (+ `PROCESS_DUP_HANDLE` for handle data;
   without it the dump has no handle table) and calls `MiniDumpWriteDump` into a new file
   (`<exe>.<pid>.<kind>.<UTC stamp>.dmp`) in the folder, a typed `Schema::Path { Dir, Write }` the host checks
   against the window's grants. Nothing is injected into or changed in the target. Access denied is a typed
   `denied` failure with the "Restart as administrator" hint. The result names the file under `file`, a new key of the
   tool output contract (`bundle`, `dir`, `file`), so the window may open it. `processes` lists processes
   (Toolhelp). Other platforms: the tool is Windows-only and answers "not available on this platform".
   - **Not done: elevated capture through ADR 0017's elevated process.** That process must not write files at paths
     it receives, and `MiniDumpWriteDump` writes to a handle. The planned design: an elevated operation opens the
     target with the read rights and duplicates that handle into the unelevated capture process (whose pid the host
     vouches for), which writes the dump. Until then the user restarts IMT elevated, as for ETW (ADR 0019).
10. **Shell and UI.** Commands (one line each in the command table): `dumps_sniff`, `dumps_open` (a path the user
    chose; grant-checked, canonical), `dumps_list`, `dumps_close`, `dumps_overview`, `dumps_threads`,
    `dumps_stack`, `dumps_modules`, `dumps_findings`, `dumps_umdh_diff`, `dumps_umdh_stack`, `dumps_umdh_tree`,
    `dumps_markers`, `dumps_explain`; every error is a `CmdError`. The DTOs live in `imt_dump::api` and are generated
    into `app/src/app/core/generated/dumps.ts` with ts-rs (ADR 0016), so the contract has one definition. Documents
    open as their own views (Dump, UMDH), are dropped like captures (sniffed first, the rest go to the log
    import), saved in projects under `ui.dumps`, and appear on the log timeline as a marker at the dump's (or
    snapshot's file) time. The views open on Findings. "Explain with AI" sends the engine's compact, redacted summary
    (`dumps_explain`: no memory contents, paths cut to file names, no thread names, at most 6,000 characters) to the
    chat, like ADR 0019.
11. **The Windows plugin's log importer stays for now.** It turns a minidump inside an opened folder into log
    records on the timeline; it duplicates part of the reading (a stack scan and an exception table). Folding it into
    the dumps plugin (a log importer over `imt-dump`'s model) is a follow-up.

## Consequences

- A malformed or hostile dump can at worst crash or exhaust the reader process, which the supervisor restarts and
  quarantines; it can neither write files nor reach the network. A full dump of many gigabytes opens in seconds and
  in tens of megabytes of memory, because only the metadata and the stacks are read.
- Stack walks without symbols use frame pointers and scanning: on x64 Windows (no frame pointers by default) the
  stacks are candidates marked `scan` until symbols with CFI exist; PDB names improve labels, not the walk.
  `imt-symbols` can add DbgHelp's unwinding later behind the same provider seam.
- UMDH's format is matched against generated samples and the documented output; real logs from other UMDH versions
  may need the reader adjusted (unknown lines are skipped, never fatal).
- Managed (.NET) stacks are not shown yet; the dump says so and points to SOS.
- Testing: unit tests per module, proptest "never panics" properties for both readers and the wire decoder, three
  fuzz targets (`fuzz_dump_minidump`, `fuzz_dump_umdh`, `fuzz_dump_wire`), synthetic minidumps (x86/x64, mini and
  full, hang and crash) from `imt_plugin_dumps::synth`, UMDH samples from `samples/generate.py`, a Windows-only test
  that writes real dumps of a child process with `MiniDumpWriteDump` and names their frames from the test binary's
  own PDB, end-to-end tests through the real plugin processes (`imt-plugins/tests/dumps.rs`), and a Playwright spec
  with the mock backend.
