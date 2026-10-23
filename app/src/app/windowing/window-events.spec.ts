const tauri = vi.hoisted(() => {
  const unlisten = vi.fn();
  return {
    unlisten,
    listen: vi.fn(async (_event: string, handler: (e: { payload: unknown }) => void, _options?: unknown) => {
      handler({ payload: { n: 1 } });
      return unlisten;
    }),
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: tauri.listen }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'w-7' }) }));

import { currentWindowLabel, listenHere } from './window-events';

describe('window events', () => {
  it('reads the current window label', async () => {
    expect(await currentWindowLabel()).toBe('w-7');
  });

  it('listens with this window as the target and passes the payload', async () => {
    const handler = vi.fn();
    const unlisten = await listenHere('debrief://open', handler);
    expect(tauri.listen).toHaveBeenCalledWith('debrief://open', expect.any(Function), { target: 'w-7' });
    expect(handler).toHaveBeenCalledWith({ n: 1 });
    unlisten();
    expect(tauri.unlisten).toHaveBeenCalled();
  });
});
