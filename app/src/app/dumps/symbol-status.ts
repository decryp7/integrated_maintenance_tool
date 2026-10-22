import { DumpSymbolProblems, DumpSymbolStatus } from '../core/api.types';

/** What a module's symbols are, in a word (the Modules table's cell). */
export function symbolStatusLabel(s: DumpSymbolStatus): string {
  switch (s.kind) {
    case 'loaded':
      return $localize`:@@dumps.symbols.loaded:Loaded`;
    case 'notNeeded':
      return $localize`:@@dumps.symbols.notNeeded:Not needed`;
    case 'noIdentity':
      return $localize`:@@dumps.symbols.noIdentity:No PDB identity`;
    case 'missing':
      return $localize`:@@dumps.symbols.missing:Missing`;
    case 'serversDisabled':
      return $localize`:@@dumps.symbols.serversDisabled:Missing (downloads off)`;
    case 'notOnServer':
      return $localize`:@@dumps.symbols.notOnServer:Not on the servers`;
    case 'mismatch':
      return $localize`:@@dumps.symbols.mismatch:Another build`;
    case 'unreadable':
      return $localize`:@@dumps.symbols.unreadable:Unreadable`;
    case 'downloadFailed':
      return $localize`:@@dumps.symbols.downloadFailed:Download failed`;
    case 'downloading':
      return $localize`:@@dumps.symbols.downloading:Downloading…`;
  }
}

/** What to do about it (`null` when nothing is wrong); the engine's detail follows where it has one. */
export function symbolStatusHint(s: DumpSymbolStatus): string | null {
  const detail = 'detail' in s && s.detail ? ` ${s.detail}` : '';
  switch (s.kind) {
    case 'loaded':
    case 'notNeeded':
      return null;
    case 'noIdentity':
      return $localize`:@@dumps.symbols.noIdentityHint:The dump records no PDB name, GUID and age for this module, so no PDB can be matched to it.`;
    case 'missing':
      return $localize`:@@dumps.symbols.missingHint:No PDB with this module's name, GUID and age is in the symbol folders or the cache. Add its PDB to a symbol folder and open the dump again.`;
    case 'serversDisabled':
      return $localize`:@@dumps.symbols.serversDisabledHint:No PDB with this module's identity was found locally. Turn on symbol servers in Settings > Symbols, or add the PDB to a symbol folder, and open the dump again.`;
    case 'notOnServer':
      return $localize`:@@dumps.symbols.notOnServerHint:None of the symbol servers has this build's PDB. Add it to a symbol folder, or add the server that has it in Settings > Symbols.`;
    case 'mismatch':
      return $localize`:@@dumps.symbols.mismatchHint:A PDB of this name was found, but of another build, so it was not used. Get the PDB that was built with this module.` + detail;
    case 'unreadable':
      return $localize`:@@dumps.symbols.unreadableHint:A PDB with this module's identity was found but could not be read. Replace the file and open the dump again.` + detail;
    case 'downloadFailed':
      return $localize`:@@dumps.symbols.downloadFailedHint:Downloading the PDB failed. Check the network and the symbol servers in Settings > Symbols, then open the dump again.` + detail;
    case 'downloading':
      return $localize`:@@dumps.symbols.downloadingHint:The PDB is being downloaded (see the Jobs pane, where it can be cancelled). The dump updates when it arrives.`;
  }
}

/** How many modules have no symbols (yet), over all causes. */
export function symbolProblemTotal(p: DumpSymbolProblems): number {
  return p.missing + p.serversDisabled + p.notOnServer + p.mismatch + p.unreadable + p.downloadFailed + p.noIdentity + p.downloading;
}

/** One line per cause that occurred. */
export function symbolProblemLines(p: DumpSymbolProblems): string[] {
  const lines: string[] = [];
  if (p.serversDisabled) lines.push($localize`:@@dumps.symbols.line.serversDisabled:${p.serversDisabled}:count: without symbols: downloads are off (Settings > Symbols).`);
  if (p.missing) lines.push($localize`:@@dumps.symbols.line.missing:${p.missing}:count: without symbols: no matching PDB in the symbol folders or the cache.`);
  if (p.notOnServer) lines.push($localize`:@@dumps.symbols.line.notOnServer:${p.notOnServer}:count: not on any symbol server.`);
  if (p.mismatch) lines.push($localize`:@@dumps.symbols.line.mismatch:${p.mismatch}:count: only found as another build's PDB.`);
  if (p.unreadable) lines.push($localize`:@@dumps.symbols.line.unreadable:${p.unreadable}:count: with a PDB that could not be read.`);
  if (p.downloadFailed) lines.push($localize`:@@dumps.symbols.line.downloadFailed:${p.downloadFailed}:count: whose download failed.`);
  if (p.downloading) lines.push($localize`:@@dumps.symbols.line.downloading:${p.downloading}:count: still downloading from the symbol servers.`);
  if (p.noIdentity) lines.push($localize`:@@dumps.symbols.line.noIdentity:${p.noIdentity}:count: record no PDB identity.`);
  return lines;
}
