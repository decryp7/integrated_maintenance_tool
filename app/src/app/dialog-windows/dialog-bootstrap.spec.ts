import { TestBed } from '@angular/core/testing';
import { UiSync } from '../popout/ui-sync';
import { dialogAppConfig, dialogBaseProviders } from './dialog-bootstrap';
import { DIALOG_DEFS } from './dialog-registry';
import { BROWSER_DIALOG_RESULT_KEY, browserDialogIo } from './dialog-io';
import { HOST_COMMAND_FORWARD } from './host-commands';

describe('dialog window bootstrap', () => {
  it('builds a small provider set, plus the features the dialog names, loaded on demand', async () => {
    const base = await dialogAppConfig('about'); // tools
    const none = await dialogAppConfig('settings');
    const ai = await dialogAppConfig('ai-setup');
    expect(none.providers.length).toBe(dialogBaseProviders().length);
    expect(base.providers.length).toBeGreaterThan(none.providers.length);
    expect(ai.providers.length).toBeGreaterThan(none.providers.length);
    expect((await dialogAppConfig('no-such-dialog')).providers.length).toBe(none.providers.length);
  });

  it('only names features that exist', async () => {
    for (const d of DIALOG_DEFS) await expect(dialogAppConfig(d.id)).resolves.toBeTruthy();
  });

  it('hands commands to the host window once sync runs', () => {
    const forward = dialogBaseProviders().find((p) => (p as { provide?: unknown }).provide === HOST_COMMAND_FORWARD) as unknown as { useFactory: () => (id: string) => boolean };
    const spy = vi.spyOn(TestBed.inject(UiSync), 'runInHost').mockReturnValue(true);
    const run = TestBed.runInInjectionContext(() => forward.useFactory());
    expect(run('view.show.parsers')).toBe(true);
    expect(spy).toHaveBeenCalledWith('dialogRun', 'view.show.parsers');
    spy.mockReturnValue(false);
    expect(run('x')).toBe(false);
  });
});

describe('browser dialog io', () => {
  it('reads its input from the route and leaves the answer for tests', async () => {
    location.hash = '#/dialog?id=about&host=main&label=d-main-1&title=About&dialogInput=%7B%22a%22%3A1%7D';
    const io = browserDialogIo();
    expect(await io.take()).toEqual({ dialogId: 'about', title: 'About', modality: 'modeless', input: { a: 1 } });
    expect(await io.windowLabel()).toBe('d-main-1');
    expect(await io.workAreas()).toEqual([]);
    await expect(io.open({ dialogId: 'x', title: 't', modality: 'modal' })).rejects.toThrow();
    const close = vi.spyOn(window, 'close').mockImplementation(() => undefined);
    await io.close({ ok: 1 });
    expect(JSON.parse(localStorage.getItem(BROWSER_DIALOG_RESULT_KEY)!)).toEqual({ result: { ok: 1 } });
    expect(close).toHaveBeenCalled();
    location.hash = '#/dialog?id=a&host=main&label=d-main-1&dialogInput=nope&modality=modal';
    expect(await io.take()).toMatchObject({ modality: 'modal', input: {} });
    location.hash = '';
  });
});
