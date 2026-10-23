import { dropPoint, isTauri, onFilesDropped, pickExportPath, pickPaths, pickProfileToOpen, pickProfileToSave, Platform, saveDialogConfirmsOverwrite } from './platform';
import { at } from '../../testing/must';

// The engine shows the dialogs (`pick_paths`) and answers with the chosen paths. Tauri's IPC is stubbed at its
// window internals: the shared test setup has already loaded `@tauri-apps/api/core` (through the mocks).
const core = vi.hoisted(() => ({ invoke: vi.fn() }));
const request = (n = -1) => core.invoke.mock.calls.at(n)![1].request;
const webview = vi.hoisted(() => {
  let handler: ((e: { payload: Record<string, unknown> }) => void) | null = null;
  const unlisten = vi.fn();
  return {
    emit: (payload: Record<string, unknown>) => handler?.({ payload }),
    unlisten,
    getCurrentWebview: () => ({
      onDragDropEvent: (h: typeof handler) => {
        handler = h;
        return Promise.resolve(unlisten);
      },
    }),
  };
});
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: webview.getCurrentWebview }));

const setTauri = (on: boolean) => {
  const w = window as unknown as Record<string, unknown>;
  if (on) w['__TAURI_INTERNALS__'] = { invoke: core.invoke };
  else delete w['__TAURI_INTERNALS__'];
};

describe('platform (browser)', () => {
  it('detects the plain browser and returns demo paths', async () => {
    setTauri(false);
    expect(isTauri()).toBe(false);
    expect(await pickPaths(false)).toEqual(['/demo/extra-service.log']);
    expect(await pickPaths(true)).toEqual(['/demo/extra-folder']);
    expect(await pickProfileToOpen()).toBe('/demo/profiles/imported.toml');
    expect(await pickProfileToSave('a.toml')).toBe('a.toml');
    expect(await pickExportPath('events.csv', [])).toBe('events.csv');
    expect(await new Platform().pickExportPath('events.csv', [])).toBe('events.csv');
    // No native dialog in a browser: the app asks before replacing a file itself.
    expect(saveDialogConfirmsOverwrite()).toBe(false);
    const off = await onFilesDropped(vi.fn(), vi.fn());
    expect(off()).toBeUndefined();
  });
});

describe('platform (Tauri)', () => {
  beforeEach(() => setTauri(true));
  afterEach(() => setTauri(false));

  it('trusts the native save dialog to confirm replacing a file', () => {
    expect(new Platform().saveDialogConfirmsOverwrite()).toBe(true);
  });

  it('uses the native open dialog', async () => {
    core.invoke.mockResolvedValueOnce(['/a', '/b']).mockResolvedValueOnce(['/c']).mockResolvedValueOnce([]);
    expect(await pickPaths(false)).toEqual(['/a', '/b']);
    expect(await pickPaths(true)).toEqual(['/c']);
    expect(await pickPaths(true)).toEqual([]);
    expect(at(core.invoke.mock.calls, 1)[0]).toBe('pick_paths');
    expect(request(1)).toMatchObject({ kind: 'openFolders' });
    expect(request(0)).toMatchObject({ kind: 'openFiles' });
  });

  it('asks for an export path with one filter per format', async () => {
    core.invoke.mockResolvedValueOnce(['/out/events.clef']);
    expect(await pickExportPath('events.csv', [{ name: 'CSV', extension: 'csv' }, { name: 'CLEF', extension: 'clef' }])).toBe('/out/events.clef');
    expect(request()).toMatchObject({ kind: 'save', defaultName: 'events.csv', filters: [{ name: 'CSV', extensions: ['csv'] }, { name: 'CLEF', extensions: ['clef'] }] });
  });

  it('opens and saves profiles natively', async () => {
    core.invoke.mockResolvedValueOnce(['/p.toml']).mockResolvedValueOnce([]);
    expect(await pickProfileToOpen()).toBe('/p.toml');
    expect(await pickProfileToOpen()).toBeNull();
    core.invoke.mockResolvedValueOnce(['/out.toml']);
    expect(await pickProfileToSave('a.toml')).toBe('/out.toml');
    expect(request()).toMatchObject({ kind: 'save', defaultName: 'a.toml' });
  });

  it('reports drag-and-drop hover and drops', async () => {
    const dropped = vi.fn();
    const hover = vi.fn();
    const off = await onFilesDropped(dropped, hover);
    webview.emit({ type: 'enter' });
    webview.emit({ type: 'over' });
    webview.emit({ type: 'leave' });
    webview.emit({ type: 'drop', paths: [] });
    webview.emit({ type: 'drop', paths: ['/logs'] });
    webview.emit({ type: 'drop', paths: ['/shot.png'], position: { x: 40, y: 20 } });
    expect(hover.mock.calls.map((c) => c[0])).toEqual([true, true, false, false, false, false]);
    // Where it landed, in CSS pixels, so the shell can route it to the drop target there.
    const scale = window.devicePixelRatio || 1;
    expect(dropped.mock.calls).toEqual([
      [['/logs'], null],
      [['/shot.png'], { x: 40 / scale, y: 20 / scale }],
    ]);
    expect(off).toBe(webview.unlisten);
  });
});

describe('Platform service', () => {
  it('delegates to the host functions', async () => {
    const { Platform } = await import('./platform');
    const p = new Platform();
    expect(p.isTauri()).toBe(false);
    expect(await p.pickPaths(false)).toEqual(['/demo/extra-service.log']);
    expect(await p.pickProfileToOpen()).toBe('/demo/profiles/imported.toml');
    expect(await p.pickProfileToSave('x.toml')).toBe('x.toml');
    expect(typeof (await p.onFilesDropped(vi.fn(), vi.fn()))).toBe('function');
  });
});

describe('platform save and open dialogs for shared files', () => {
  afterEach(() => setTauri(false));

  it('returns browser placeholders, then uses the native dialogs', async () => {
    setTauri(false);
    const p = new Platform();
    expect(await p.pickSavePath('Save', 'report.html', [{ name: 'HTML', extensions: ['html'] }])).toBe('report.html');
    expect(await p.pickOpenPath('Open bundle', [{ name: 'Debrief bundle', extensions: ['debriefz'] }])).toBe('/demo/shared.debriefz');
    expect(await p.pickOpenPath('Open', [])).toBe('/demo/shared.txt');
    setTauri(true);
    core.invoke.mockResolvedValueOnce(['/out/r.pdf']);
    expect(await p.pickSavePath('Export report', 'r.pdf', [{ name: 'PDF', extensions: ['pdf'] }])).toBe('/out/r.pdf');
    expect(request()).toEqual({ kind: 'save', title: 'Export report', defaultName: 'r.pdf', filters: [{ name: 'PDF', extensions: ['pdf'] }] });
    core.invoke.mockResolvedValueOnce(['/in/case.debriefz']).mockResolvedValueOnce([]);
    expect(await p.pickOpenPath('Open bundle', [{ name: 'Debrief bundle', extensions: ['debriefz'] }])).toBe('/in/case.debriefz');
    expect(request()).toMatchObject({ kind: 'openFile', title: 'Open bundle' });
    expect(await p.pickOpenPath('Open bundle', [])).toBeNull();
  });
});

describe('platform multi-file open dialog', () => {
  afterEach(() => {
    setTauri(false);
    history.replaceState(null, '', '/');
  });

  it('returns a browser placeholder or ?mockOpen, then the native choice', async () => {
    setTauri(false);
    const p = new Platform();
    expect(await p.pickOpenPaths('Add', [{ name: 'Archives', extensions: ['zip'] }])).toEqual(['/demo/shared.zip']);
    expect(await p.pickOpenPaths('Add', [])).toEqual(['/demo/shared.txt']);
    history.replaceState(null, '', '/?mockOpen=/d/a.zip,/d/b.zip');
    expect(await p.pickOpenPaths('Add', [])).toEqual(['/d/a.zip', '/d/b.zip']);
    setTauri(true);
    core.invoke.mockResolvedValueOnce(['/a.zip', '/b.zip']).mockResolvedValueOnce(['/c.zip']).mockResolvedValueOnce([]);
    expect(await p.pickOpenPaths('Add archives', [{ name: 'Archives', extensions: ['zip'] }])).toEqual(['/a.zip', '/b.zip']);
    expect(request()).toMatchObject({ kind: 'openFiles', title: 'Add archives' });
    expect(await p.pickOpenPaths('Add', [])).toEqual(['/c.zip']);
    expect(await p.pickOpenPaths('Add', [])).toEqual([]);
  });
});

describe('dropPoint', () => {
  it('turns the OS drop position into CSS pixels', () => {
    expect(dropPoint({ x: 300, y: 150 }, 1.5)).toEqual({ x: 200, y: 100 });
    expect(dropPoint({ x: 10, y: 20 }, 0)).toEqual({ x: 10, y: 20 });
    expect(dropPoint({ x: Number.NaN, y: 1 }, 1)).toBeNull();
    expect(dropPoint(null, 1)).toBeNull();
  });
});

