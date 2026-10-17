import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, tinyBackend } from '../../testing/helpers';
import { ImtBackend } from './backend';
import { MockBackend } from './mock/mock-backend';
import { MOCK_DEMO_PATHS } from './mock/mock-import';
import { MockUxBackend } from './mock/mock-ux-backend';
import { TauriUxBackend, UxBackend } from './ux-backend';

describe('UxBackend', () => {
  it('maps every method to its Tauri command', async () => {
    const calls: [string, unknown][] = [];
    const t = new TauriUxBackend((async (cmd: string, args?: unknown) => (calls.push([cmd, args]), cmd === 'profile_history' ? [] : null)) as never);
    await t.importProgress();
    await t.cancelImport();
    await t.profileHistory();
    await t.profileVersion('a.toml');
    await t.profileTemplates();
    await t.demoPaths();
    expect(calls).toEqual([
      ['import_progress', undefined],
      ['cancel_import', undefined],
      ['profile_history', undefined],
      ['profile_version', { file: 'a.toml' }],
      ['profile_templates', undefined],
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
    const imt = tinyBackend();
    const ux = new MockUxBackend(imt);
    await imt.openPaths(['/x/new.log']);
    expect(await ux.importProgress()).toMatchObject({ running: false, done: 1, total: 1 });
    await ux.cancelImport();
    const before = (await imt.getProfile()).toml;
    await imt.saveProfile('name = "Second"\n');
    const [v] = await ux.profileHistory();
    expect((await ux.profileVersion(v.file)).toml).toBe(before);
    expect((await ux.profileTemplates()).length).toBeGreaterThan(0);
  });

  it('the mock degrades over a backend that is not the mock workspace', async () => {
    const ux = new MockUxBackend({} as ImtBackend);
    expect((await ux.importProgress()).total).toBe(0);
    await expect(ux.cancelImport()).resolves.toBeUndefined();
    expect(await ux.profileHistory()).toEqual([]);
    await expect(ux.profileVersion('x')).rejects.toThrow('no profile history');
  });

  it('the mock templates are fresh copies', async () => {
    const ux = new MockUxBackend(new MockBackend({ latency: false }));
    const [a] = await ux.profileTemplates();
    a.name = 'changed';
    expect((await ux.profileTemplates())[0].name).not.toBe('changed');
  });
});
