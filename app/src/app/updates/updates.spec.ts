import { TestBed } from '@angular/core/testing';
import { Logger } from '../core/logger';
import { MemoryLogger } from '../../testing/helpers';
import { at } from '../../testing/must';
import { DialogService } from '../ui/dialog.service';
import { MockUpdatesBackend, scenarioOf } from './mock-updates-backend';
import { TauriUpdatesBackend, UpdatesBackend } from './updates-backend';
import { UpdatesCommands } from './updates.commands';
import { progressText, releaseDate, UpdatesDialog } from './updates-dialog';
import { UpdatesStore } from './updates.store';
import { NotificationHost } from '../health/notification-host';
import { provideNotificationSource } from '../health/provide-notification-host';
import { UpdatesSource } from './updates-source';
import { toUpdateError, UPDATES_EVENT, UpdateStatus } from './updates.types';

function setup(search = '') {
  const backend = new MockUpdatesBackend(search, 1);
  TestBed.configureTestingModule({
    providers: [
      { provide: UpdatesBackend, useValue: backend },
      { provide: Logger, useValue: new MemoryLogger() },
      provideNotificationSource(UpdatesSource),
    ],
  });
  return { backend, store: TestBed.inject(UpdatesStore) };
}

const until = async (cond: () => boolean) => {
  for (let i = 0; i < 200 && !cond(); i++) await new Promise((r) => setTimeout(r, 2));
  expect(cond()).toBe(true);
};

describe('updates types', () => {
  it('turns rejections into update errors', () => {
    const e = { kind: 'offline', message: 'm', nextStep: 'n', detail: null };
    expect(toUpdateError(e)).toEqual(e);
    expect(toUpdateError({ kind: 'busy', message: 'b' })).toEqual({ kind: 'busy', message: 'b', nextStep: '', detail: null });
    expect(toUpdateError('boom')).toMatchObject({ kind: 'unknown', message: 'boom' });
    expect(toUpdateError(new Error('x'))).toMatchObject({ kind: 'unknown', message: 'x' });
    expect(toUpdateError(42).message).toBe('The request failed.');
  });

  it('formats progress and dates', () => {
    expect(progressText(1_500_000, 48_000_000)).toBe('1.4 MB of 46 MB');
    expect(progressText(500, null)).toBe('500 bytes');
    expect(releaseDate(null)).toBe('');
    expect(releaseDate('nonsense')).toBe('');
    expect(releaseDate('2026-10-17T12:00:00Z')).toContain('2026');
  });
});

describe('TauriUpdatesBackend', () => {
  it('maps every call to its command and listens to the status event', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    }) as never;
    let listened = '';
    const listen = async (event: string, handler: (p: unknown) => void) => {
      listened = event;
      handler({ currentVersion: '1' });
      return () => undefined;
    };
    const b = new TauriUpdatesBackend(call, listen);
    await b.status();
    await b.check();
    await b.download();
    await b.cancel();
    await b.installOnQuit(true);
    await b.restart();
    await b.openReleases();
    const seen: UpdateStatus[] = [];
    await b.onStatus((s) => seen.push(s));
    expect(calls).toEqual([
      ['updates_status', undefined],
      ['updates_check', undefined],
      ['updates_download', undefined],
      ['updates_status', undefined],
      ['updates_discard', undefined],
      ['updates_install_on_quit', { enabled: true }],
      ['updates_restart', undefined],
      ['updates_open_releases', undefined],
    ]);
    expect(listened).toBe(UPDATES_EVENT);
    expect(seen.length).toBe(1);
  });

  it('cancels a download in flight through its job', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => (calls.push([cmd, args]), cmd === 'updates_status' ? { downloadJob: 3 } : null)) as never;
    await new TauriUpdatesBackend(call, async () => () => undefined).cancel();
    expect(calls).toEqual([
      ['updates_status', undefined],
      ['jobs_cancel', { id: 3 }],
      ['updates_status', undefined],
    ]);
  });
});

describe('MockUpdatesBackend', () => {
  it('reads its scenario from the URL', () => {
    expect(scenarioOf('')).toBe('available');
    expect(scenarioOf('?mockUpdate=offline')).toBe('offline');
    expect(scenarioOf('?mockUpdate=nonsense')).toBe('available');
  });

  it('refuses steps that do not apply', async () => {
    const b = new MockUpdatesBackend('', 1);
    await expect(b.download()).rejects.toMatchObject({ kind: 'busy' });
    await expect(b.cancel()).rejects.toMatchObject({ kind: 'busy' });
    await expect(b.installOnQuit(true)).rejects.toMatchObject({ kind: 'busy' });
    await expect(b.restart()).rejects.toMatchObject({ kind: 'busy' });
    const u = new MockUpdatesBackend('?mockUpdate=unavailable', 1);
    await expect(u.check()).rejects.toMatchObject({ kind: 'notAvailable' });
    expect((await u.status()).endpointHost).toBeNull();
  });
});

describe('UpdatesStore', () => {
  it('checks, downloads with progress, and prepares the restart', async () => {
    const { backend, store } = setup();
    await until(() => store.status() !== null);
    expect(store.phase()?.kind).toBe('idle');
    expect(store.available()).toBe(true);
    await store.check();
    expect(store.phase()?.kind).toBe('available');
    // Found while no dialog is open: announced once.
    expect(store.notice()).toBe('0.2.0');
    store.dismissNotice();
    await store.download();
    await until(() => store.progress() !== null);
    expect(store.progress()).toBeGreaterThanOrEqual(0);
    await until(() => store.phase()?.kind === 'ready');
    await store.setInstallOnQuit(true);
    expect(store.phase()).toMatchObject({ kind: 'ready', installOnQuit: true });
    await store.restart();
    expect(store.phase()?.kind).toBe('installing');
    expect(backend.calls).toEqual(['check', 'download', 'installOnQuit:true', 'restart']);
    expect(store.notice()).toBeNull();
    await store.check();
    expect(store.notice()).toBeNull();
  });

  it('cancels a download and keeps the update', async () => {
    const { store } = setup();
    await store.check();
    await store.download();
    await store.cancel();
    expect(store.phase()?.kind).toBe('available');
  });

  it('shows engine failures in the phase and request failures as error', async () => {
    const { store } = setup('?mockUpdate=offline');
    await store.check();
    expect(store.phase()).toMatchObject({ kind: 'failed', error: { kind: 'offline' } });
    await store.download();
    expect(store.error()?.kind).toBe('busy');
    await store.openReleases();
    expect(store.error()).toBeNull();
  });

  it('reports a build without updates', async () => {
    const { store } = setup('?mockUpdate=unavailable');
    await store.refresh();
    expect(store.available()).toBe(false);
    await store.check();
    expect(store.error()?.kind).toBe('notAvailable');
  });

  it('does not announce while the dialog is open', async () => {
    const { store } = setup();
    store.setDialogOpen(true);
    await store.check();
    expect(store.notice()).toBeNull();
    store.setDialogOpen(false);
    expect(store.notice()).toBeNull();
  });
});

describe('UpdatesDialog', () => {
  function render(search = '') {
    const ctx = setup(search);
    const fixture = TestBed.createComponent(UpdatesDialog);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const settle = async () => {
      await fixture.whenStable();
      fixture.detectChanges();
    };
    const button = (name: string) => [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === name);
    return { ...ctx, fixture, el, settle, button };
  }

  it('walks through available, downloading and ready', async () => {
    const { store, el, settle, button } = render();
    await store.check();
    await settle();
    expect(el.querySelector('[data-testid=updates-available]')?.textContent).toContain('Debrief 0.2.0 is available');
    const notes = el.querySelector('[data-testid=updates-notes]')!;
    // Notes are text: markup in them is shown, not rendered.
    expect(notes.textContent).toContain('<b>Faster</b>');
    expect(notes.querySelector('b')).toBeNull();
    expect(el.textContent).toContain('46 MB');
    expect(el.querySelector('[data-testid=updates-auto]')?.textContent).toContain('dev.decryptology.net');
    button('Download')!.click();
    await until(() => store.phase()?.kind === 'downloading');
    await settle();
    expect(el.querySelector('progress')).not.toBeNull();
    expect(button('Cancel')).toBeTruthy();
    await until(() => store.phase()?.kind === 'ready');
    await settle();
    expect(el.querySelector('[data-testid=updates-ready]')).not.toBeNull();
    const box = el.querySelector<HTMLInputElement>('input[type=checkbox]')!;
    box.checked = true;
    box.dispatchEvent(new Event('change'));
    await until(() => store.phase()?.kind === 'ready' && (store.phase() as { installOnQuit: boolean }).installOnQuit);
    button('Restart to update')!.click();
    await until(() => store.phase()?.kind === 'installing');
    await settle();
    expect(el.textContent).toContain('Restarting to install Debrief 0.2.0');
  });

  it('shows typed errors with their next step and a retry', async () => {
    const { store, el, settle, button, backend } = render('?mockUpdate=diskFull');
    await store.check();
    await store.download();
    await until(() => store.phase()?.kind === 'failed');
    await settle();
    const err = el.querySelector('[data-testid=updates-error]')!;
    expect(err.getAttribute('data-kind')).toBe('diskFull');
    expect(err.textContent).toContain('Free some disk space');
    button('Try the download again')!.click();
    button('Open releases page')!.click();
    await settle();
    expect(backend.calls).toContain('openReleases');
  });

  it('offers a new check after a refused signature', async () => {
    const { store, el, settle, button } = render('?mockUpdate=signatureInvalid');
    await store.check();
    await store.download();
    await until(() => store.phase()?.kind === 'failed');
    await settle();
    expect(el.querySelector('[data-testid=updates-error]')?.getAttribute('data-kind')).toBe('signatureInvalid');
    expect(button('Try the download again')).toBeUndefined();
    expect(button('Check again')).toBeTruthy();
  });

  it('says when this build cannot update', async () => {
    const { el, settle, store } = render('?mockUpdate=unavailable');
    await until(() => store.status() !== null);
    await settle();
    expect(el.querySelector('[data-testid=updates-unavailable]')).not.toBeNull();
    expect(el.querySelector('[data-testid=updates-auto]')).toBeNull();
  });

  it('shows up to date', async () => {
    const { el, settle, store, button } = render('?mockUpdate=upToDate');
    await store.check();
    await settle();
    expect(el.querySelector('[data-testid=updates-up-to-date]')).not.toBeNull();
    button('Check again')!.click();
  });
});

describe('UpdatesCommands and toast', () => {
  it('opens the dialog once and checks', async () => {
    const { store, backend } = setup();
    const dialogs = TestBed.inject(DialogService);
    const commands = TestBed.inject(UpdatesCommands);
    expect(at(commands.commands, 0).id).toBe('help.checkForUpdates');
    await at(commands.commands, 0).run({} as never);
    commands.open();
    expect(dialogs.stack().filter((d) => d.component === UpdatesDialog).length).toBe(1);
    await until(() => store.phase()?.kind === 'available');
    expect(backend.calls.filter((c) => c === 'check').length).toBeGreaterThanOrEqual(1);
  });

  it('the toast announces an update and opens the dialog', async () => {
    const { store } = setup();
    const fixture = TestBed.createComponent(NotificationHost);
    await store.check();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('[data-testid=updates-toast]')?.textContent).toContain('Debrief 0.2.0 is available');
    el.querySelector<HTMLButtonElement>('[data-testid=notification-action]')!.click();
    fixture.detectChanges();
    expect(TestBed.inject(DialogService).stack().some((d) => d.component === UpdatesDialog)).toBe(true);
    expect(el.querySelector('[data-testid=updates-toast]')).toBeNull();
  });
});
