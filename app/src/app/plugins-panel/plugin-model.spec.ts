import { ProcessMemory } from '../settings/performance/performance.types';
import { InstalledPluginDto } from '../core/api.types';
import { packageRow } from './plugins-table';
import { contributionKind, memoryView, provideGroups, statusLabel } from './plugin-model';

const row = (patch: Partial<ProcessMemory>): ProcessMemory => ({ process: { kind: 'plugin', pluginId: 'p' }, name: null, capMb: 1000, overrideMb: null, usedMb: null, peakMb: null, state: null, ...patch });

describe('plugin model', () => {
  it('groups contributions by kind with a count per kind', () => {
    expect(contributionKind('importer:evtx')).toBe('importer');
    expect(contributionKind('hook')).toBe('hook');
    const groups = provideGroups(['importer:evtx', 'marker:a', 'marker:b', 'hook', 'weird:x']);
    expect(groups.map((g) => [g.kind, g.items.length, g.icon])).toEqual([
      ['importer', 1, 'file'],
      ['marker', 2, 'alert'],
      ['hook', 1, 'live'],
      ['weird', 1, 'puzzle'],
    ]);
    expect(provideGroups([])).toEqual([]);
  });

  it('shows memory used of the cap, calm unless near or over it', () => {
    expect(memoryView(undefined)).toBeNull();
    expect(memoryView(row({ usedMb: 420, capMb: 1024 }))).toEqual({ text: '420 / 1024 MB', percent: 41, level: 'ok', note: null });
    expect(memoryView(row({ peakMb: 900 }))?.text).toBe('900 / 1000 MB');
    expect(memoryView(row({ capMb: 512 }))).toMatchObject({ text: '512 MB cap', percent: null });
    expect(memoryView(row({ usedMb: 950, state: 'warned' }))).toMatchObject({ level: 'warn', note: 'near its cap' });
    expect(memoryView(row({ usedMb: 2000, state: 'stopped' }))).toMatchObject({ level: 'danger', percent: 100, note: 'stopped over its cap' });
    expect(memoryView(row({ usedMb: 5, capMb: 0 }))?.percent).toBe(0);
  });

  it('words every status', () => {
    expect(['active', 'disabled', 'quarantined', 'inactive'].map((s) => statusLabel(s as never))).toEqual(['Running', 'Disabled', 'Quarantined', 'Not running']);
  });
});

describe('packageRow', () => {
  const pkg = (patch: Partial<InstalledPluginDto>): InstalledPluginDto => ({
    id: 'p.x', name: 'X', version: '1.0.0', publisher: 'P', description: '', signature: { kind: 'unsigned', signer: null }, warning: null,
    requested: [], granted: [], enabled: true, state: 'ready', problem: null, installedAtMs: 0, ...patch,
  });

  it('maps a package the engine is not running to a table row', () => {
    expect(packageRow(pkg({ state: 'running' })).status).toBe('active');
    expect(packageRow(pkg({ state: 'disabled' })).status).toBe('disabled');
    const quarantined = packageRow(pkg({ state: 'quarantined', problem: 'Crashed' }));
    expect(quarantined).toMatchObject({ status: 'quarantined', lastError: 'Crashed' });
    const waiting = packageRow(pkg({ state: 'pendingRestart' }));
    expect(waiting).toMatchObject({ status: 'inactive', inactiveReason: 'pending restart' });
    expect(packageRow(pkg({ state: 'unavailable', problem: 'Needs a newer IMT' })).inactiveReason).toBe('Needs a newer IMT');
    expect(packageRow(pkg({ warning: 'Developer mode' })).warnings).toEqual(['Developer mode']);
  });
});
