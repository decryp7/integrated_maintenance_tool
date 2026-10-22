import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, tinyBackend } from '../../testing/helpers';
import { DebriefBackend } from './backend';
import { MOCK_DEMO_PATHS } from './mock/mock-import';
import { MockUxBackend } from './mock/mock-ux-backend';
import { TauriUxBackend, UxBackend } from './ux-backend';
import { at } from '../../testing/must';

describe('UxBackend', () => {
  it('maps every method to its Tauri command', async () => {
    const calls: [string, unknown][] = [];
    const t = new TauriUxBackend((async (cmd: string, args?: unknown) => (calls.push([cmd, args]), cmd === 'profile_history' ? [] : cmd === 'jobs_list' ? { jobs: [{ id: 4, kind: 'import', state: 'running' }], paused: [] } : null)) as never);
    await t.importProgress();
    await t.cancelImport();
    await t.profileHistory();
    await t.profileVersion('a.toml');
    await t.demoPaths();
    expect(calls).toEqual([
      ['import_progress', undefined],
      ['jobs_list', undefined],
      ['jobs_cancel', { id: 4 }],
      ['profile_history', undefined],
      ['profile_version', { file: 'a.toml' }],
      ['demo_paths', undefined],
    ]);
  });

  it('is provided in root: the mock over the mock workspace in a browser', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackendBare() });
    const ux = TestBed.inject(UxBackend);
    expect((await ux.importProgress()).running).toBe(false);
    expect(await ux.demoPaths()).toEqual([...MOCK_DEMO_PATHS]);
  });

  it('the mock reads progress, history and versions from the mock workspace', async () => {
    const debrief = tinyBackend();
    const ux = new MockUxBackend(debrief);
    await debrief.openPaths(['/x/new.log']);
    expect(await ux.importProgress()).toMatchObject({ running: false, done: 1, total: 1 });
    await ux.cancelImport();
    const before = (await debrief.getProfile()).toml;
    await debrief.saveProfile('name = "Second"\n');
    const v = at(await ux.profileHistory(), 0);
    expect((await ux.profileVersion(v.file)).toml).toBe(before);
  });

  it('the mock degrades over a backend that is not the mock workspace', async () => {
    const ux = new MockUxBackend({} as DebriefBackend);
    expect((await ux.importProgress()).total).toBe(0);
    await expect(ux.cancelImport()).resolves.toBeUndefined();
    expect(await ux.profileHistory()).toEqual([]);
    await expect(ux.profileVersion('x')).rejects.toThrow('no profile history');
  });
});
