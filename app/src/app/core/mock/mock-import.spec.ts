import { FILE_EXISTS_SUFFIX, ImportProgress } from '../api.types';
import { tinyDataset } from '../../../testing/helpers';
import { MockBackend } from './mock-backend';
import { isMockDemoPath, MOCK_DEMO_PATHS, MockFiles, MockImportSim, MockProfileHistory, NOT_IMPORTED_REASON, SLOW_FILE_BYTES, SLOW_FILES_PER_PATH, slowImportFiles } from './mock-import';
import { mockOptionsFromSearch } from './mock-options';

import { at } from '../../../testing/must';
import { MockJobs, mockJobs, resetMockJobs } from '../../jobs/mock-jobs';
describe('mock import, files and profile history', () => {
  it('a slow import reports progress file by file and stops after a cancel', async () => {
    const sim = new MockImportSim(5);
    expect(sim.progress().running).toBe(false);
    sim.cancel(); // nothing running: ignored
    const files = ['/a/1', '/a/2', '/a/3', '/a/4'];
    const read: string[] = [];
    const run = sim.run(files, (f) => {
      read.push(f);
      if (read.length === 2) sim.cancel();
    });
    expect(sim.progress()).toMatchObject({ running: true, total: 4, bytesTotal: 4 * SLOW_FILE_BYTES, current: '/a/1' });
    const result = await run;
    expect(result).toEqual({ read: ['/a/1', '/a/2'], skipped: ['/a/3', '/a/4'] });
    expect(sim.progress()).toMatchObject({ running: false, done: 2, cancelled: true, current: null, etaMs: null, bytesDone: 2 * SLOW_FILE_BYTES });
  });

  it('with slow discovery it finds files (a growing count), detects formats, then reads; Cancel ends any phase', async () => {
    const sim = new MockImportSim(1, undefined, 1);
    const phases: string[] = [];
    const found: number[] = [];
    const seen = (p: ImportProgress) => {
      if (phases.at(-1) !== p.phase) phases.push(p.phase);
      if (p.phase === 'finding') found.push(p.done);
    };
    await sim.run(['/a/1', '/a/2'], () => undefined, seen);
    expect(phases).toEqual(['finding', 'importing']);
    expect(found.length).toBeGreaterThan(3);
    expect(found.every((n, i) => i === 0 || n > found[i - 1]!)).toBe(true);
    // Cancelled while finding: nothing is read and everything is skipped.
    const cancelled = new MockImportSim(1, undefined, 5);
    const run = cancelled.run(['/a/1', '/a/2'], () => expect.unreachable('read after a cancel'));
    cancelled.cancel();
    expect(await run).toEqual({ read: [], skipped: ['/a/1', '/a/2'] });
    expect(cancelled.progress()).toMatchObject({ running: false, cancelled: true, total: 2, done: 0 });
  });

  it('reads ?mockSlowDiscovery from the URL (capped)', () => {
    expect(mockOptionsFromSearch('?mockSlowDiscovery=2000').slowDiscoveryMs).toBe(2000);
    expect(mockOptionsFromSearch('?mockSlowDiscovery=999999').slowDiscoveryMs).toBe(120_000);
    expect(mockOptionsFromSearch('').slowDiscoveryMs).toBeUndefined();
  });

  it('estimates the time left while it runs', async () => {
    const sim = new MockImportSim(5);
    const etas: (number | null)[] = [];
    await sim.run(['/a', '/b'], () => etas.push(sim.progress().etaMs));
    expect(etas[0]).toBeNull(); // nothing read yet: no estimate
    expect(typeof etas[1]).toBe('number');
    expect(sim.progress()).toMatchObject({ done: 2, cancelled: false });
  });

  it('pretends each opened path is a folder of files', () => {
    expect(slowImportFiles(['/x/', '/y'])).toHaveLength(2 * SLOW_FILES_PER_PATH);
    expect(slowImportFiles(['/x/'])[0]).toBe('/x/part-1.log');
  });

  it('refuses to replace a written file without overwrite', () => {
    const files = new MockFiles();
    files.write('/o.csv', false);
    expect(files.has('/o.csv')).toBe(true);
    expect(() => files.write('/o.csv', undefined)).toThrow(`/o.csv${FILE_EXISTS_SUFFIX}`);
    files.write('/o.csv', true);
    files.clear();
    expect(files.has('/o.csv')).toBe(false);
  });

  it('keeps the newest 20 profile versions and refuses unsafe names', () => {
    const h = new MockProfileHistory();
    for (let i = 0; i < 25; i++) h.archive(`name = "v${i}"`, `v${i}`, Date.UTC(2026, 9, 17, 9, 0, i, 5));
    const list = h.list();
    expect(list).toHaveLength(20);
    expect(list[0]).toMatchObject({ name: 'v24', savedAt: '20261017-090024005', savedAtMs: Date.UTC(2026, 9, 17, 9, 0, 24, 5) });
    expect(h.get(at(list, 0).file).toml).toBe('name = "v24"');
    expect(() => h.get('../x')).toThrow('invalid version');
    expect(() => h.get('nope')).toThrow('no such version');
  });

  it('knows the demo paths', () => {
    expect(MOCK_DEMO_PATHS.every(isMockDemoPath)).toBe(true);
    expect(isMockDemoPath('/demo/samples')).toBe(true);
    expect(isMockDemoPath('/demo/other')).toBe(false);
  });

  it('reads ?mockSlowImport from the URL (capped)', () => {
    expect(mockOptionsFromSearch('?mockSlowImport=600').slowImportMs).toBe(600);
    expect(mockOptionsFromSearch('?mockSlowImport=999999').slowImportMs).toBe(120_000);
    expect(mockOptionsFromSearch('?mockSlowImport=x').slowImportMs).toBeUndefined();
  });

  describe('MockBackend', () => {
    it('opens the demo into an empty workspace, and merges it once into a non-empty one', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [] });
      await b.clearWorkspace();
      const demo = await b.openPaths([...MOCK_DEMO_PATHS]);
      expect(demo.sources.length).toBeGreaterThan(2);
      const tiny = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [] });
      const merged = await tiny.openPaths([at(MOCK_DEMO_PATHS, 0)]);
      expect(merged.sources.length).toBe(2 + demo.sources.length);
      expect((await tiny.openPaths([at(MOCK_DEMO_PATHS, 0)])).sources.length).toBe(merged.sources.length);
    });

    it('a slow open runs as an import job whose progress carries files, bytes and the current file', async () => {
      const jobs = new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() });
      resetMockJobs(jobs);
      try {
        const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], slowImportMs: 5 });
        const open = b.openPaths(['/big']);
        await new Promise((r) => setTimeout(r, 12));
        const running = (await jobs.list()).jobs.find((j) => j.kind === 'import' && j.state === 'running');
        expect(running?.title).toBe('Open /big');
        expect(running?.progress).toMatchObject({ total: SLOW_FILES_PER_PATH, bytesTotal: SLOW_FILES_PER_PATH * SLOW_FILE_BYTES, current: expect.stringContaining('/big/part-') });
        await open;
        const done = (await jobs.list()).jobs.find((j) => j.kind === 'import');
        expect(done?.state).toBe('done');
        expect(done?.progress).toMatchObject({ done: SLOW_FILES_PER_PATH, bytesDone: SLOW_FILES_PER_PATH * SLOW_FILE_BYTES });
      } finally {
        resetMockJobs();
      }
    });

    it('a slow open is cancellable: unread files are listed as not imported', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], slowImportMs: 5 });
      const open = b.openPaths(['/big']);
      await new Promise((r) => setTimeout(r, 12));
      expect((await b.importProgress()).running).toBe(true);
      await b.cancelImport();
      const summary = await open;
      const notImported = summary.unrecognized.filter((u) => u.reason === NOT_IMPORTED_REASON);
      expect(notImported.length).toBeGreaterThan(0);
      expect(summary.sources.length + notImported.length).toBe(2 + SLOW_FILES_PER_PATH);
      expect((await b.importProgress()).cancelled).toBe(true);
      // Opening again lists a skipped file only once.
      await b.openPaths([]);
      expect((await b.workspaceSummary()).unrecognized.filter((u) => u.reason === NOT_IMPORTED_REASON)).toHaveLength(notImported.length);
    });

    it('archives the replaced profile and refuses a second export to the same file', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [] });
      await b.saveProfile('name = "Next"\n');
      expect((await b.profileHistory())[0]?.name).not.toBe('Next');
      expect((await b.profileVersion(at(await b.profileHistory(), 0).file)).path).toBeNull();
      const request = { query: { from: null, to: null, levels: [], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [] }, format: 'csv', path: '/out/e.csv' };
      const first = await b.exportEvents(request);
      expect(typeof first).toBe('number');
      await expect(b.exportEvents(request)).rejects.toContain(FILE_EXISTS_SUFFIX);
      const end = await mockJobs().wait(await b.exportEvents({ ...request, overwrite: true }));
      expect(end.result).toEqual({ type: 'file', path: '/out/e.csv' });
      expect(end.message).toMatch(/^Exported \d+ events\.$/);
    });
  });
});
