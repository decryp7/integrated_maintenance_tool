import { DialogService } from '../ui/dialog.service';
import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Logger } from '../core/logger';
import { MemoryLogger } from '../../testing/helpers';
import { CommandRegistry } from '../extensions/command-registry';
import { ErrorSnapshotsView } from './error-snapshots-view';
import { ErrorSnapshotService } from './error-snapshot.service';
import { ExportTarget } from './export-target';
import { MockSnapshotBackend } from './mock/mock-snapshot-backend';
import { SnapshotBackend } from './snapshot-backend';
import { SnapshotsStore } from './snapshots.store';
import { NotificationHost } from '../health/notification-host';
import { provideNotificationSource } from '../health/provide-notification-host';
import { SnapshotSource } from './snapshot-source';
import { CapturedImage, SNAPSHOT_NOTICE } from './snapshot.types';
import { at } from '../../testing/must';
import { mockJobs } from '../jobs/mock-jobs';
import { NotificationsStore } from '../notifications/notifications.store';

const IMAGE: CapturedImage = { base64: 'iVBORw0KGgo=', mime: 'image/png', bytes: 8 };

class FakeTarget extends ExportTarget {
  path: string | null = '/out/diag.zip';
  override async pickZip() {
    return this.path;
  }
}

function setup() {
  const backend = new MockSnapshotBackend();
  const target = new FakeTarget();
  const run = vi.fn(async (_id: string) => true);
  TestBed.configureTestingModule({
    providers: [
      { provide: SnapshotBackend, useValue: backend },
      { provide: ExportTarget, useValue: target },
      { provide: Logger, useValue: new MemoryLogger() },
      { provide: CommandRegistry, useValue: { run } },
    ],
  });
  return { backend, target, run };
}

async function flush(f: ComponentFixture<unknown>) {
  for (let i = 0; i < 4; i++) {
    await f.whenStable();
    f.detectChanges();
  }
}

describe('ErrorSnapshotsView', () => {
  it('lists snapshots with time, reason and message, and opens one with its context', async () => {
    const { backend } = setup();
    await backend.save(IMAGE, { reason: 'panel-failed', message: 'Panel broke', stack: 'at x', view: 'Grid', manual: false });
    await backend.save(IMAGE, { reason: 'manual', message: 'manual', manual: true });
    const f = TestBed.createComponent(ErrorSnapshotsView);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelectorAll('[data-testid=snapshot-card]').length).toBe(2);
    expect(el.textContent).toContain('panel-failed');
    expect(el.querySelectorAll('img').length).toBe(2);
    expect(el.textContent).toContain(SNAPSHOT_NOTICE);

    (el.querySelectorAll('button.thumb')[1] as HTMLButtonElement).click();
    await flush(f);
    const detail = el.querySelector('section[aria-label="Snapshot details"]')!;
    expect(detail.textContent).toContain('Panel broke');
    expect(detail.textContent).toContain('at x');
    expect(detail.textContent).toContain('Grid');
    (detail.querySelector('button.ghost') as HTMLButtonElement).click();
    await flush(f);
    expect(el.querySelector('section[aria-label="Snapshot details"]')).toBeNull();
  });

  it('shows an empty state, then deletes, selects, exports and opens the folder', async () => {
    const { backend, target, run } = setup();
    const f = TestBed.createComponent(ErrorSnapshotsView);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.textContent).toContain('No error snapshots yet');
    expect(el.textContent).toMatch(/press (Ctrl\+Shift\+F12|⌘⇧F12) or Capture now/);
    const exportButton = () => [...el.querySelectorAll('button')].find((b) => b.textContent?.includes('Export diagnostics'))!;
    // Nothing captured: only the empty-state sentence and Capture now, the primary action; no claim that a screenshot was saved (E6).
    expect(exportButton()).toBeUndefined();
    expect(el.textContent).not.toContain('Debrief saved a screenshot');
    expect(el.querySelector('label.all')).toBeNull();
    expect([...el.querySelectorAll('button')].find((b) => b.textContent?.includes('Capture now'))?.classList.contains('primary')).toBe(true);

    const a = await backend.save(IMAGE, { reason: 'manual', message: 'one', manual: false });
    const b = await backend.save(IMAGE, { reason: 'manual', message: 'two', manual: false });
    const store = TestBed.inject(SnapshotsStore);
    await store.refresh();
    await flush(f);
    expect(exportButton().classList.contains('primary')).toBe(true);
    expect([...el.querySelectorAll('button')].find((b) => b.textContent?.includes('Capture now'))?.classList.contains('primary')).toBe(false);
    expect(el.textContent).toContain('Debrief saved a screenshot');

    // Export all when nothing is ticked.
    const exported = vi.spyOn(backend, 'exportDiagnostics');
    exportButton().click();
    await flush(f);
    expect(backend.exports[0]).toEqual({ path: '/out/diag.zip', ids: [b.id, a.id] });
    expect(el.textContent).toContain('Exporting 2 snapshots to /out/diag.zip; a message tells you when the file is ready.');
    expect(at(TestBed.inject(NotificationsStore).toasts(), 0).title).toBe('Exporting diagnostics — running in Jobs');
    const end = await mockJobs().wait(await at(exported.mock.results, 0).value);
    expect(end.result).toEqual({ type: 'file', path: '/out/diag.zip' });
    expect(end.message).toBe('5 files zipped.');

    // Export only the ticked one, over the first export after confirming.
    (el.querySelectorAll('input.pick')[1] as HTMLInputElement).click();
    exportButton().click();
    await flush(f);
    const dialogs = TestBed.inject(DialogService);
    expect(at(dialogs.stack(), 0).options.title).toBe('Replace file?');
    at(dialogs.stack(), 0).ref.close(true);
    await flush(f);
    expect(at(backend.exports, 1).ids).toEqual([a.id]);

    // A cancelled dialog exports nothing.
    target.path = null;
    exportButton().click();
    await flush(f);
    expect(backend.exports.length).toBe(2);

    const button = (text: string) => [...el.querySelectorAll('button')].find((x) => x.textContent?.includes(text))!;
    button('Delete selected').click();
    await flush(f);
    expect((await backend.list()).map((i) => i.id)).toEqual([b.id]);
    expect(el.querySelectorAll('[data-testid=snapshot-card]').length).toBe(1);

    button('Open folder').click();
    await flush(f);
    expect(backend.folderOpened).toBe(1);
    expect(el.textContent).toContain('/mock/logs/snapshots');

    button('Settings…').click();
    expect(run).toHaveBeenCalledWith('settings.diagnostics');
  });

  it('select all, delete from the detail and errors from the backend', async () => {
    const { backend } = setup();
    await backend.save(IMAGE, { reason: 'manual', message: 'one', manual: false });
    await backend.save(IMAGE, { reason: 'manual', message: 'two', manual: false });
    const f = TestBed.createComponent(ErrorSnapshotsView);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    const store = TestBed.inject(SnapshotsStore);
    (el.querySelector('.all input') as HTMLInputElement).click();
    expect(store.selected().length).toBe(2);
    expect(store.allSelected()).toBe(true);
    store.toggleAll();
    expect(store.selected()).toEqual([]);

    store.show(at(store.items(), 0).id);
    await flush(f);
    (el.querySelector('.dhead .btn:not(.ghost)') as HTMLButtonElement).click();
    await flush(f);
    expect(store.items().length).toBe(1);

    const list = backend.list.bind(backend);
    backend.list = async () => {
      throw new Error('list failed');
    };
    await store.refresh();
    await flush(f);
    expect(el.querySelector('[role=alert]')?.textContent).toContain('list failed');
    backend.openFolder = async () => {
      throw new Error('no file manager');
    };
    await store.openFolder();
    expect(store.error()).toContain('no file manager');
    backend.remove = async () => {
      throw new Error('locked');
    };
    backend.list = list;
    await store.remove(['x']);
    expect(store.error()).toContain('locked');
    backend.exportDiagnostics = async () => {
      throw new Error('disk full');
    };
    await store.exportSelected();
    expect(store.error()).toContain('disk full');
  });

  it('refreshes when a new snapshot is saved and can capture on demand', async () => {
    const { backend } = setup();
    const f = TestBed.createComponent(ErrorSnapshotsView);
    await flush(f);
    expect(f.nativeElement.querySelectorAll('[data-testid=snapshot-card]').length).toBe(0);
    await backend.save(IMAGE, { reason: 'manual', message: 'new', manual: false });
    const service = TestBed.inject(ErrorSnapshotService);
    service.saved.update((n) => n + 1);
    await flush(f);
    expect(f.nativeElement.querySelectorAll('[data-testid=snapshot-card]').length).toBe(1);
    const manual = vi.spyOn(service, 'manual').mockResolvedValue('id');
    await TestBed.inject(SnapshotsStore).captureNow();
    expect(manual).toHaveBeenCalled();
  });
});

describe('the snapshot notification', () => {
  @Component({ selector: 'debrief-host', imports: [NotificationHost], providers: [provideNotificationSource(SnapshotSource)], template: '<debrief-notification-host />' })
  class Host {}

  it('says "Something went wrong in <panel>." with Details folded; Details holds the screenshot notice and View snapshot (E4)', async () => {
    const { backend, run } = setup();
    const f = TestBed.createComponent(Host);
    const service = TestBed.inject(ErrorSnapshotService);
    f.detectChanges();
    expect(f.nativeElement.querySelector('[data-testid=notification]')).toBeNull();
    service.toast.set({ kind: 'captured', id: 'a', firstTime: true, where: 'Timeline' });
    await flush(f);
    const toast: HTMLElement = f.nativeElement.querySelector('[data-testid=notification]');
    expect(toast.textContent).toContain('Something went wrong in Timeline.');
    expect(toast.textContent).not.toContain('screenshot');
    expect(toast.querySelector('[data-testid=notification-details]')).toBeNull();
    const toggle = toast.querySelector('[data-testid=notification-details-toggle]') as HTMLButtonElement;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    toggle.click();
    await flush(f);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toast.querySelector('[data-testid=notification-details]')?.textContent).toContain(SNAPSHOT_NOTICE);
    // A new toast starts folded again.
    service.toast.set({ kind: 'captured', id: 'b', firstTime: false });
    await flush(f);
    expect(toast.textContent).toContain('Something went wrong.');
    expect(toast.querySelector('[data-testid=notification-details]')).toBeNull();
    toggle.click();
    await flush(f);
    [...toast.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'View snapshot')!.click();
    expect(run).toHaveBeenCalledWith('diagnostics.snapshots.show');
    expect(service.toast()).toBeNull();
    expect(backend.folderOpened).toBe(0);
  });

  it('a manual capture says so with View; Details links to the folder and the settings', async () => {
    const { backend, run } = setup();
    const f = TestBed.createComponent(Host);
    const service = TestBed.inject(ErrorSnapshotService);
    service.toast.set({ kind: 'captured', id: 'm', firstTime: false, manual: true });
    await flush(f);
    expect(f.nativeElement.textContent).toContain('Diagnostic screenshot captured.');
    expect(f.nativeElement.textContent).not.toContain('went wrong');
    service.toast.set({ kind: 'captured', id: 'a', firstTime: true });
    await flush(f);
    (f.nativeElement.querySelector('[data-testid=notification-details-toggle]') as HTMLButtonElement).click();
    await flush(f);
    const notice: HTMLElement = f.nativeElement.querySelector('[data-testid=notification-details]');
    expect(notice.textContent).toContain(SNAPSHOT_NOTICE);
    const links = [...(f.nativeElement.querySelector('[data-testid=notification]') as HTMLElement).querySelectorAll('button')];
    const link = (label: string) => links.find((b) => b.textContent === label) as HTMLButtonElement;
    link('Open folder').click();
    link('Settings…').click();
    expect(backend.folderOpened).toBe(1);
    expect(run).toHaveBeenCalledWith('settings.diagnostics');
    backend.openFolder = async () => {
      throw new Error('x');
    };
    link('Open folder').click();
    (f.nativeElement.querySelector('.x') as HTMLButtonElement).click();
    await flush(f);
    expect(f.nativeElement.querySelector('[data-testid=notification]')).toBeNull();
  });

  it('reports a failed manual capture', async () => {
    setup();
    const f = TestBed.createComponent(Host);
    TestBed.inject(ErrorSnapshotService).toast.set({ kind: 'failed', id: null, firstTime: false });
    await flush(f);
    expect(f.nativeElement.textContent).toContain('could not be captured');
  });
});
