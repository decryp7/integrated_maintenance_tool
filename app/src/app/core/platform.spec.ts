import { isTauri, onFilesDropped, pickExportPath, pickPaths, pickProfileToOpen, pickProfileToSave, Platform, saveDialogConfirmsOverwrite } from './platform';

const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
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
vi.mock('@tauri-apps/plugin-dialog', () => dialog);
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: webview.getCurrentWebview }));

const setTauri = (on: boolean) => {
  const w = window as unknown as Record<string, unknown>;
  if (on) w['__TAURI_INTERNALS__'] = {};
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
    dialog.open.mockResolvedValueOnce(['/a', '/b']).mockResolvedValueOnce('/c').mockResolvedValueOnce(null);
    expect(await pickPaths(false)).toEqual(['/a', '/b']);
    expect(await pickPaths(true)).toEqual(['/c']);
    expect(await pickPaths(true)).toEqual([]);
    expect(dialog.open.mock.calls[1][0]).toMatchObject({ directory: true, multiple: true });
  });

  it('asks for an export path with one filter per format', async () => {
    dialog.save.mockResolvedValueOnce('/out/events.clef');
    expect(await pickExportPath('events.csv', [{ name: 'CSV', extension: 'csv' }, { name: 'CLEF', extension: 'clef' }])).toBe('/out/events.clef');
    expect(dialog.save.mock.calls.at(-1)![0]).toMatchObject({ defaultPath: 'events.csv', filters: [{ name: 'CSV', extensions: ['csv'] }, { name: 'CLEF', extensions: ['clef'] }] });
  });

  it('opens and saves profiles natively', async () => {
    dialog.open.mockResolvedValueOnce('/p.toml').mockResolvedValueOnce(['/x', '/y']);
    expect(await pickProfileToOpen()).toBe('/p.toml');
    expect(await pickProfileToOpen()).toBeNull();
    dialog.save.mockResolvedValueOnce('/out.toml');
    expect(await pickProfileToSave('a.toml')).toBe('/out.toml');
    expect(dialog.save.mock.calls[0][0]).toMatchObject({ defaultPath: 'a.toml' });
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
    expect(hover.mock.calls.map((c) => c[0])).toEqual([true, true, false, false, false]);
    expect(dropped).toHaveBeenCalledExactlyOnceWith(['/logs']);
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
    expect(await p.pickOpenPath('Open bundle', [{ name: 'IMT bundle', extensions: ['imtz'] }])).toBe('/demo/shared.imtz');
    expect(await p.pickOpenPath('Open', [])).toBe('/demo/shared.txt');
    setTauri(true);
    dialog.save.mockResolvedValueOnce('/out/r.pdf');
    expect(await p.pickSavePath('Export report', 'r.pdf', [{ name: 'PDF', extensions: ['pdf'] }])).toBe('/out/r.pdf');
    expect(dialog.save.mock.calls.at(-1)![0]).toEqual({ title: 'Export report', defaultPath: 'r.pdf', filters: [{ name: 'PDF', extensions: ['pdf'] }] });
    dialog.open.mockResolvedValueOnce('/in/case.imtz').mockResolvedValueOnce(['/a', '/b']);
    expect(await p.pickOpenPath('Open bundle', [{ name: 'IMT bundle', extensions: ['imtz'] }])).toBe('/in/case.imtz');
    expect(dialog.open.mock.calls.at(-1)![0]).toMatchObject({ multiple: false, directory: false, title: 'Open bundle' });
    expect(await p.pickOpenPath('Open bundle', [])).toBeNull();
  });
});
