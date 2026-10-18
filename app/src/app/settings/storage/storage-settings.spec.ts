import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideFakeDock, provideTinyBackend, settle, text } from '../../../testing/helpers';
import { CommandRegistry } from '../../extensions/command-registry';
import { DialogService } from '../../ui/dialog.service';
import { EngineSettingsStore } from '../engine-settings.store';
import { MockStorageBackend } from './mock-storage-backend';
import { StorageBackend } from './storage-backend';
import { categoryLabel, resultLine, sizeLabel, StorageSettings } from './storage-settings';
import { CLEAR_CACHE_COMMAND, STORAGE_ID } from './storage.types';

async function setup(backend = new MockStorageBackend()) {
  TestBed.configureTestingModule({ providers: [{ provide: StorageBackend, useValue: backend }] });
  const fixture = TestBed.createComponent(StorageSettings);
  await settle(fixture);
  return { fixture, backend, dialogs: TestBed.inject(DialogService) };
}

const row = (fixture: Parameters<typeof el>[0], id: string) => el(fixture, `tr[data-category="${id}"]`);

describe('StorageSettings', () => {
  it('lists every category with its kind, size, files and location', async () => {
    const { fixture } = await setup();
    expect(all(fixture, 'tbody tr')).toHaveLength(8);
    const logs = row(fixture, 'logs');
    expect(logs.textContent).toContain('IMT logs');
    expect(logs.textContent).toContain('data');
    expect(logs.textContent).toContain('6 MB');
    expect(logs.textContent).toContain('6 files');
    expect(logs.textContent).toContain("Today's log is kept.");
    expect(row(fixture, 'webview-cache').textContent).toContain('cache');
    // Cache rows are cleared together; only data rows have their own Clear button.
    expect(row(fixture, 'webview-cache').querySelector('.act button')).toBeNull();
    expect(row(fixture, 'logs').querySelector<HTMLButtonElement>('.act button')?.getAttribute('aria-label')).toBe('Clear… (IMT logs)');
    // Nothing clearable now: disabled.
    expect(row(fixture, 'sessions').querySelector<HTMLButtonElement>('.act button')?.disabled).toBe(true);
    expect(el(fixture, 'tfoot').textContent).toContain('GB');
  });

  it('"Clear cache" confirms what goes and how much, then reports the result', async () => {
    const { fixture, backend, dialogs } = await setup();
    const clear = vi.spyOn(backend, 'clear');
    buttonByText(fixture, 'Clear cache…').click();
    await settle(fixture);
    const dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('Clear cache?');
    expect(String(dlg?.options.inputs?.['message'])).toContain('267 MB');
    expect(String(dlg?.options.inputs?.['detail'])).toContain('Web view cache — 45 MB');
    expect(String(dlg?.options.inputs?.['detail'])).not.toContain('logs');
    dlg?.ref.close(true);
    await settle(fixture);
    expect(clear).toHaveBeenCalledWith(['bundle-extracts', 'webview-cache', 'ai-runtimes-outdated']);
    const result = el(fixture, '[data-testid=storage-result]').textContent ?? '';
    expect(result).toContain('Freed 267 MB.');
    expect(result).toContain('Web view cache: freed 45 MB, kept 1 (in use by a running program).');
    expect(buttonByText(fixture, 'Clear cache…').disabled).toBe(true);
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Freed');
  });

  it('a data category needs its own confirmation, and cancelling keeps it', async () => {
    const { fixture, backend, dialogs } = await setup();
    const clear = vi.spyOn(backend, 'clear');
    row(fixture, 'logs').querySelector<HTMLButtonElement>('.act button')?.click();
    await settle(fixture);
    let dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('Remove IMT logs?');
    expect(String(dlg?.options.inputs?.['message'])).toContain('permanently deletes 5 MB');
    expect(dlg?.options.inputs?.['danger']).toBe(true);
    dlg?.ref.close();
    await settle(fixture);
    expect(clear).not.toHaveBeenCalled();

    row(fixture, 'logs').querySelector<HTMLButtonElement>('.act button')?.click();
    await settle(fixture);
    dlg = dialogs.stack().at(-1);
    dlg?.ref.close(true);
    await settle(fixture);
    expect(clear).toHaveBeenCalledWith(['logs']);
    expect(text(fixture)).toContain('IMT logs: freed 5 MB, kept 1 (IMT is writing to it).');
  });

  it('shows a failed measurement with Retry', async () => {
    const { fixture } = await setup(new MockStorageBackend('?mockStorageFail=1'));
    expect(el(fixture, '[role=alert]').textContent).toContain("Could not measure IMT's storage");
    expect(all(fixture, 'tbody tr')).toHaveLength(0);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, 'tbody tr')).toHaveLength(8);
  });

  it('opens a folder and shows a failure inline with Retry', async () => {
    const backend = new MockStorageBackend();
    const { fixture } = await setup(backend);
    const reveal = vi.spyOn(backend, 'reveal').mockRejectedValueOnce('explorer is missing');
    const show = row(fixture, 'logs').querySelector<HTMLButtonElement>('.where button');
    expect(show?.getAttribute('aria-label')).toBe('Show in folder (IMT logs)');
    show?.click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('explorer is missing');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(reveal).toHaveBeenCalledTimes(2);
    expect(text(fixture)).toContain('Opened C:\\Users\\demo');
  });

  it('formats sizes, labels and result lines', () => {
    expect(sizeLabel(2048, false)).toBe('2 KB');
    expect(sizeLabel(2048, true)).toBe('≥ 2 KB');
    expect(categoryLabel({ id: 'stores', label: 'x' })).toBe('Persistent stores');
    expect(categoryLabel({ id: 'future' as never, label: 'Future thing' })).toBe('Future thing');
    const base = { id: 'logs' as const, freedBytes: 1024, removedFiles: 1, skipped: [], skippedCount: 0, error: null };
    expect(resultLine(base, 'IMT logs')).toBe('IMT logs: freed 1 KB.');
    expect(resultLine({ ...base, error: 'no access' }, 'IMT logs')).toBe('IMT logs: no access');
  });
});

describe('Clear cache command', () => {
  it('opens Settings at the Storage section', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(), { provide: StorageBackend, useValue: new MockStorageBackend() }] });
    const registry = TestBed.inject(CommandRegistry);
    await settle();
    expect(await registry.run(CLEAR_CACHE_COMMAND)).toBe(true);
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Settings');
    expect(TestBed.inject(EngineSettingsStore).selected()).toBe(STORAGE_ID);
  });
});
