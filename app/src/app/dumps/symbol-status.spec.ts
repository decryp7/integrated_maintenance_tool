import { describe, expect, it } from 'vitest';
import { DumpSymbolProblems, DumpSymbolStatus } from '../core/api.types';
import { symbolProblemLines, symbolProblemTotal, symbolStatusHint, symbolStatusLabel } from './symbol-status';

const NONE: DumpSymbolProblems = { missing: 0, serversDisabled: 0, notOnServer: 0, mismatch: 0, unreadable: 0, downloadFailed: 0, noIdentity: 0 };

describe('symbol status', () => {
  it('labels every status and gives the next step for each problem', () => {
    const kinds: DumpSymbolStatus[] = [
      { kind: 'loaded' },
      { kind: 'notNeeded' },
      { kind: 'noIdentity' },
      { kind: 'missing' },
      { kind: 'serversDisabled' },
      { kind: 'notOnServer' },
      { kind: 'mismatch', detail: 'GUID differs' },
      { kind: 'unreadable', detail: 'not a PDB' },
      { kind: 'downloadFailed', detail: 'timed out' },
    ];
    expect(new Set(kinds.map(symbolStatusLabel)).size).toBe(kinds.length);
    expect(symbolStatusHint({ kind: 'loaded' })).toBeNull();
    expect(symbolStatusHint({ kind: 'notNeeded' })).toBeNull();
    for (const k of kinds.slice(2)) expect(symbolStatusHint(k), k.kind).toBeTruthy();
    expect(symbolStatusHint({ kind: 'mismatch', detail: 'GUID differs' })).toContain('GUID differs');
    expect(symbolStatusHint({ kind: 'downloadFailed', detail: 'timed out' })).toContain('timed out');
    expect(symbolStatusHint({ kind: 'unreadable', detail: '' })).not.toMatch(/\s$/);
    expect(symbolStatusHint({ kind: 'serversDisabled' })).toContain('Settings > Symbols');
  });

  it('counts and lists the causes that occurred', () => {
    expect(symbolProblemTotal(NONE)).toBe(0);
    expect(symbolProblemLines(NONE)).toEqual([]);
    const all = { missing: 1, serversDisabled: 2, notOnServer: 3, mismatch: 4, unreadable: 5, downloadFailed: 6, noIdentity: 7 };
    expect(symbolProblemTotal(all)).toBe(28);
    expect(symbolProblemLines(all)).toHaveLength(7);
    expect(symbolProblemLines({ ...NONE, notOnServer: 3 })).toEqual(['3 not on any symbol server.']);
  });
});
