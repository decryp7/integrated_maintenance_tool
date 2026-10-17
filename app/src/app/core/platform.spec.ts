import { isTauri, onFilesDropped, pickPaths, pickProfileToOpen, pickProfileToSave } from './platform';

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
    const off = await onFilesDropped(vi.fn(), vi.fn());
    expect(off()).toBeUndefined();
  });
});

describe('platform (Tauri)', () => {
  beforeEach(() => setTauri(true));
  afterEach(() => setTauri(false));

  it('uses the native open dialog', async () => {
    dialog.open.mockResolvedValueOnce(['/a', '/b']).mockResolvedValueOnce('/c').mockResolvedValueOnce(null);
    expect(await pickPaths(false)).toEqual(['/a', '/b']);
    expect(await pickPaths(true)).toEqual(['/c']);
    expect(await pickPaths(true)).toEqual([]);
    expect(dialog.open.mock.calls[1][0]).toMatchObject({ directory: true, multiple: true });
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
