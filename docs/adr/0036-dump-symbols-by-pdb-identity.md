# 36. Dump symbols by PDB identity: matching, servers, cache and status

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Amends: [ADR 0023](0023-dumps-and-umdh.md) (symbols come from PDBs matched by identity, not by file name; UMDH is no longer symbolised from folders)
- Related: ADR 0017 (out-of-process plugins), ADR 0022 (native symbols), ADR 0023 (dumps and UMDH), ADR 0031
  (settings schemas in Rust), AGENTS.md (parse, don't validate; bounded resources; degrade, don't fail)

## Context

The dumps plugin named a dump's frames from PDBs found in folders by file name. A PDB of the same name but another
build gives wrong function names that look right, which is worse than no names. A UMDH log has no PDB identity at all,
yet the old code symbolised it from the same folders. There was no way to fetch missing PDBs, and a module without
names did not say why.

## Decision

1. **A PDB is matched by identity, never by file name.** `imt_symbols::id::PdbKey` (name, GUID, age) is parsed once
   from the untrusted text a dump records (`PdbName` accepts one safe path component ending in `.pdb`; the id is 32 hex
   digits of GUID and a hex age). It is the only way a cache path or a server address is built. `imt_symbols::msf`
   reads the identity a PDB file records, from a bounded header check (MSF 7.0, the PDB info stream), so a candidate
   is accepted only when its GUID and age equal the module's. A file of the same name and another identity is
   `Mismatch`, not used.
2. **UMDH is not symbolised from folders.** A UMDH log keeps the names UMDH wrote; it records no identity to match.
3. **Per-module `SymbolStatus`** (`imt_dump::model`): `NotNeeded`, `Loaded`, `NoIdentity`, `Missing`,
   `ServersDisabled`, `NotOnServer`, `Mismatch`, `Unreadable`, `DownloadFailed`, the last three with a bounded detail
   text. The wire (`DumpSymbolStatus`, `DumpSymbolProblems` in `dumps.ts`) carries it; the Modules table shows each
   status with its next step, and the Details card counts the causes.
4. **Symbol servers are off by default** and are the user's: Settings ▸ Symbols (`serversEnabled`, `serverUrls` up to
   8 HTTPS addresses, `downloadLimit`, `maxPdbSize`, `concurrent`, `cacheMaxSize`, `cacheFolder`). Every address is
   checked when saved (`https://` only), and nothing is requested while downloads are off. A request sends the PDB's
   name, GUID and age (they are in the dump), over HTTPS with the platform's certificate verifier, bounded in time and
   size; the answer is written to a temporary file and renamed into the cache only when it is a PDB recording the
   identity asked for.
5. **The host downloads; the plugin reads.** `dumps_open` reads the dump with the cache as an extra symbol folder
   (`ImportOptions::symbol_folders`, at most 4, absolute paths only), asks the servers (`imt_symbols::servers::get_all`:
   each PDB once, a process-wide `Gate` bounding concurrent downloads) for the modules that are `Missing`, and reads
   the dump again when any PDB arrived. The others get the status the outcome says. At most 256 PDBs are asked per
   dump. A dump that cannot be read a second time keeps its first reading.
6. **The cache is a symstore-layout folder** (`<cache>/<pdb>/<key>/<pdb>`) bounded by `cacheMaxSize`: after a download
   the least recently used PDBs are removed (`imt_symbols::cache::prune`; a hit touches the file).
7. **Testing without the network**: a fake symbol server (`imt_symbols::testing`, a loopback HTTP listener) serves
   synthetic MSF files, so the download, mismatch, size-limit and cache paths are tested for real; the shell's logic is
   tested with a scripted `Fetcher`.

## Consequences

- A wrong-build PDB can no longer name frames; the UI says "another build" and what to get.
- Opening a dump with downloads on can take as long as the downloads (each bounded by `downloadLimit`); it runs in the
  open command's blocking task. Making it a cancellable job with progress is future work.
- The dumps plugin's own symbol-folder setting stays; the cache is added by the host and is not a setting of the plugin.
- Windows-only `dbghelp` code is behind the `dbghelp` feature of `imt-symbols`; `imt-dump` and the dumps plugin depend
  on the pure part only.
