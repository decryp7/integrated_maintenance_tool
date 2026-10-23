import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { ProjectService } from '../projects/project.service';
import { MockToolsOptions } from '../tools/mock/mock-tools-backend';
import { mockTools, provideToolsTesting } from '../tools/mock/test-support';
import { LifecycleBanners } from './lifecycle-banners';
import { LifecycleCommands } from './lifecycle-commands';
import { LifecycleService } from './lifecycle.service';
import { SessionService } from './session.service';
import { TitleExtras } from './title-extras';
import { UI_STATE_CONTRIBUTIONS } from './ui-state';
import { CoreUiState } from './core-ui-state';
import { WindowingBackend } from '../windowing/windowing-backend';
import { DiagnosticsExport } from '../diagnostics/diagnostics-export.service';

async function setup(options: Partial<MockToolsOptions> = {}) {
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(options), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(CoreUiState).contributions }] });
  const life = TestBed.inject(LifecycleService);
  await life.init();
  TestBed.inject(SessionService).start();
  return { life, tools: mockTools(), store: TestBed.inject(WorkspaceStore), session: TestBed.inject(SessionService) };
}

describe('LifecycleService', () => {
  it('reads status and elevation', async () => {
    const { life } = await setup({ safeMode: true, elevated: true });
    expect(life.safeMode()).toBe(true);
    expect(life.elevated()).toBe(true);
    expect(life.canRestartElevated()).toBe(false);
    expect(life.showCrashBanner()).toBe(false);
    expect(life.sessionAvailable()).toBe(false);
  });

  it('offers a restart as administrator on Windows when not elevated only', async () => {
    expect((await setup()).life.canRestartElevated()).toBe(true);
    TestBed.resetTestingModule();
    expect((await setup({ platform: 'macos' })).life.canRestartElevated()).toBe(false);
  });

  it('exposes the platform reported by the backend, independent of elevation', async () => {
    const win = (await setup({ elevated: true })).life;
    expect(win.platform()).toBe('windows');
    expect(win.isWindows()).toBe(true);
    expect(win.canRestartElevated()).toBe(false);
    TestBed.resetTestingModule();
    const linux = (await setup({ platform: 'linux' })).life;
    expect(linux.platform()).toBe('linux');
    expect(linux.isWindows()).toBe(false);
  });

  it('keeps defaults when the backend cannot answer', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const tools = mockTools();
    vi.spyOn(tools, 'appStatus').mockRejectedValue(new Error('x'));
    vi.spyOn(tools, 'elevation').mockRejectedValue(new Error('x'));
    const life = TestBed.inject(LifecycleService);
    await life.init();
    expect(life.status()).toBeNull();
    expect(life.safeMode()).toBe(false);
    expect(life.canRestartElevated()).toBe(false);
    expect(life.platform()).toBeNull();
    expect(life.isWindows()).toBe(false);
  });

  it('holds back autosave while the crash offer is pending and restores the last session', async () => {
    const { life, session, store, tools } = await setup({ previousCrash: true });
    expect(life.showCrashBanner()).toBe(true);
    expect(session.enabled()).toBe(false);
    await life.restoreLastSession();
    expect(life.showCrashBanner()).toBe(false);
    expect(session.enabled()).toBe(true);
    expect(store.summary()).not.toBeNull();
    expect(TestBed.inject(ProjectService).path()).toBeNull();
    expect(tools.calls.some((c) => c.startsWith('saveProject'))).toBe(false);
  });

  it('says so when there is no session and keeps the offer when opening it fails', async () => {
    const { life, store, tools } = await setup({ previousCrash: true });
    vi.spyOn(tools, 'lastSession').mockResolvedValueOnce(null);
    await life.restoreLastSession();
    expect(store.notice()).toContain('no saved session');
    expect(life.showCrashBanner()).toBe(false);
    TestBed.resetTestingModule();
    const again = await setup({ previousCrash: true });
    vi.spyOn(again.tools, 'openProject').mockRejectedValue(new Error('corrupt'));
    await again.life.restoreLastSession();
    expect(again.life.showCrashBanner()).toBe(true);
    expect(again.store.error()).toContain('corrupt');
    vi.spyOn(again.tools, 'lastSession').mockRejectedValue(new Error('io'));
    await again.life.restoreLastSession();
    expect(again.store.error()).toContain('Could not restore the last session: io');
  });

  it('reopens the other windows only after a successful restore; their failure does not undo it', async () => {
    const { life, store, tools } = await setup({ previousCrash: true });
    const restore = vi.spyOn(TestBed.inject(WindowingBackend), 'restoreWindows');
    vi.spyOn(tools, 'lastSession').mockResolvedValueOnce(null);
    await life.restoreLastSession();
    expect(restore).not.toHaveBeenCalled();
    await life.restoreLastSession();
    expect(restore).toHaveBeenCalledTimes(1);
    restore.mockRejectedValueOnce('no display');
    await life.restoreLastSession();
    expect(store.error()).toBe('Could not reopen the other windows: no display');
    expect(life.showCrashBanner()).toBe(false);
    expect(store.summary()).not.toBeNull();
  });

  it('dismisses the crash banner and re-enables autosave', async () => {
    const { life, session } = await setup({ previousCrash: true });
    life.dismissCrash();
    expect(life.showCrashBanner()).toBe(false);
    expect(session.enabled()).toBe(true);
  });

  it('restarts in safe mode, normally, keeping the session, and elevated (saving the session first)', async () => {
    const { life, tools } = await setup({ safeMode: true });
    await life.startInSafeMode();
    await life.restartNormally();
    await life.restartKeepingSession();
    await life.restartElevated();
    expect(tools.calls.filter((c) => c !== 'saveSession')).toEqual([
      'restartApp:true:',
      'restartApp:false:',
      'restartApp:true:session',
      'restartElevated:session',
    ]);
    expect(tools.calls.indexOf('saveSession')).toBeLessThan(tools.calls.indexOf('restartElevated:session'));
  });

  it('restarts without reopening when the session cannot be saved, and reports failures', async () => {
    const { life, tools, store } = await setup();
    vi.spyOn(tools, 'saveSession').mockRejectedValue(new Error('disk'));
    await life.restartKeepingSession();
    expect(tools.calls).toContain('restartApp:false:');
    vi.spyOn(tools, 'restartElevated').mockRejectedValue(new Error('UAC refused'));
    await life.restartElevated();
    expect(store.error()).toBe('Could not restart as administrator: UAC refused');
    vi.spyOn(tools, 'lastSession').mockResolvedValue(null);
    vi.spyOn(tools, 'saveSession').mockResolvedValue();
    await life.restartKeepingSession();
    expect(tools.calls).toContain('restartApp:false:');
  });
});

describe('LifecycleCommands', () => {
  it('offers elevation and safe mode restarts depending on the state', async () => {
    const { life, tools } = await setup();
    const c = TestBed.inject(LifecycleCommands).commands;
    const by = (id: string) => c.find((x) => x.id === id)!;
    const ctx = {} as never;
    expect(by('app.restartElevated').when!(ctx)).toBe(true);
    expect(by('app.restartSafeMode').when!(ctx)).toBe(true);
    expect(by('app.restartNormally').when!(ctx)).toBe(false);
    await by('app.restartElevated').run(ctx);
    await by('app.restartSafeMode').run(ctx);
    await by('app.restartNormally').run(ctx);
    expect(tools.calls.some((x) => x.startsWith('restartElevated'))).toBe(true);
    life.status.set({ ...life.status()!, safeMode: true });
    expect(by('app.restartSafeMode').when!(ctx)).toBe(false);
    expect(by('app.restartNormally').when!(ctx)).toBe(true);
  });
});

describe('LifecycleBanners and TitleExtras', () => {
  it('shows the safe-mode banner with Restart normally', async () => {
    const { tools } = await setup({ safeMode: true });
    const f = TestBed.createComponent(LifecycleBanners);
    await settle(f);
    expect(el(f, '[data-testid=safe-mode-banner]').textContent).toContain('Running in safe mode: plugins and saved profile are off');
    buttonByText(f, 'Restart normally').click();
    await settle(f);
    expect(tools.calls).toContain('restartApp:false:');
    expect(fixtureHas(f, 'crash-banner')).toBe(false);
  });

  it('shows the crash banner with restore, safe mode and dismiss', async () => {
    const { tools, life } = await setup({ previousCrash: true });
    const f = TestBed.createComponent(LifecycleBanners);
    await settle(f);
    expect(el(f, '[data-testid=crash-banner]').textContent).toContain("Debrief didn't close properly last time");
    buttonByText(f, 'Start in safe mode').click();
    await settle(f);
    expect(tools.calls).toContain('restartApp:true:');
    buttonByText(f, 'Restore last session').click();
    await settle(f);
    expect(fixtureHas(f, 'crash-banner')).toBe(false);
    life['crashHandled'].set(false);
    await settle(f);
    buttonByText(f, 'Dismiss').click();
    await settle(f);
    expect(fixtureHas(f, 'crash-banner')).toBe(false);
  });

  it('offers Save diagnostics in the safe-mode and crash banners', async () => {
    await setup({ safeMode: true, previousCrash: true });
    const exporter = TestBed.inject(DiagnosticsExport);
    const save = vi.spyOn(exporter, 'save').mockResolvedValue();
    const f = TestBed.createComponent(LifecycleBanners);
    await settle(f);
    for (const banner of ['safe-mode-banner', 'crash-banner']) {
      const button = el(f, `[data-testid=${banner}] [data-testid=save-diagnostics]`) as HTMLButtonElement;
      expect(button.textContent).toContain('Save diagnostics…');
      button.click();
    }
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('shows the outcome of saving diagnostics and lets the user dismiss it', async () => {
    await setup({ safeMode: true });
    const exporter = TestBed.inject(DiagnosticsExport);
    const f = TestBed.createComponent(LifecycleBanners);
    exporter.status.set({ kind: 'saved', text: 'Saved diagnostics (9 files) to /x.zip' });
    await settle(f);
    expect(el(f, '[data-testid=diagnostics-export-status]').textContent).toContain('Saved diagnostics (9 files)');
    exporter.status.set({ kind: 'failed', text: 'Could not save the diagnostics: no' });
    await settle(f);
    expect(el(f, '[data-testid=diagnostics-export-status]').getAttribute('role')).toBe('alert');
    (el(f, '[data-testid=diagnostics-export-status] .ghost') as HTMLButtonElement).click();
    await settle(f);
    expect(fixtureHas(f, 'diagnostics-export-status')).toBe(false);
  });

  it('disables restore when there is no session file', async () => {
    const { life } = await setup({ previousCrash: true });
    life.status.set({ ...life.status()!, sessionAvailable: false });
    const f = TestBed.createComponent(LifecycleBanners);
    await settle(f);
    expect(buttonByText(f, 'Restore last session').disabled).toBe(true);
  });

  it('lists project paths that were not found', async () => {
    await setup();
    const projects = TestBed.inject(ProjectService);
    projects.missing.set(['/a/one.log']);
    const f = TestBed.createComponent(LifecycleBanners);
    await settle(f);
    expect(text(f)).toContain('1 path of this project could not be found: one.log');
    projects.missing.set(['/a/1.log', '/a/2.log', '/a/3.log', '/a/4.log', '/a/5.log']);
    await settle(f);
    expect(text(f)).toContain('5 paths');
    expect(text(f)).toContain('1.log, 2.log, 3.log and 2 more');
    buttonByText(f, 'Dismiss').click();
    await settle(f);
    expect(fixtureHas(f, 'missing-banner')).toBe(false);
  });

  it('shows the administrator badge and the project name with an unsaved dot', async () => {
    const { store } = await setup({ elevated: true });
    const f = TestBed.createComponent(TitleExtras);
    await settle(f);
    expect(el(f, '[data-testid=admin-badge]').textContent).toBe('Administrator');
    expect(el(f, '[data-testid=project-title]').textContent).toContain('Untitled');
    expect(all(f, '.unsaved')).toHaveLength(0);
    store.patchQuery({ text: 'changed' }); store.setTextInput('changed');
    await settle(f);
    expect(all(f, '.unsaved')).toHaveLength(1);
    TestBed.resetTestingModule();
    await setup();
    const g = TestBed.createComponent(TitleExtras);
    await settle(g);
    expect(fixtureHas(g, 'admin-badge')).toBe(false);
  });
});

function fixtureHas(f: { nativeElement: HTMLElement }, id: string): boolean {
  return !!f.nativeElement.querySelector(`[data-testid=${id}]`);
}
