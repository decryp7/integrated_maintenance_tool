import { TestBed } from '@angular/core/testing';
import { all, el, settle, typeInto } from '../../../testing/helpers';
import { DialogService } from '../../ui/dialog.service';
import { MockStorageBackend } from './mock-storage-backend';
import { StorageBackend } from './storage-backend';
import { StorageStores } from './stores-block';

async function setup(search = '') {
  const backend = new MockStorageBackend(search);
  TestBed.configureTestingModule({ providers: [{ provide: StorageBackend, useValue: backend }] });
  const fixture = TestBed.createComponent(StorageStores);
  await settle(fixture);
  return { fixture, backend, dialogs: TestBed.inject(DialogService) };
}

const rows = (f: Parameters<typeof el>[0]) => all(f, 'tbody tr');
const button = (f: Parameters<typeof el>[0], id: string) => el<HTMLButtonElement>(f, `[data-testid=${id}]`);

describe('StorageStores', () => {
  it('lists the folders newest first with size, last use and a state badge in words', async () => {
    const { fixture } = await setup();
    expect(rows(fixture)).toHaveLength(5);
    const states = rows(fixture).map((r) => r.querySelector('.badge')?.textContent?.trim());
    expect(states).toEqual(['Open', 'Kept for session restore', 'In use by another Debrief', 'Orphaned', 'Orphaned']);
    const first = rows(fixture)[0]!;
    expect(first.textContent).toContain('store-7f3a91c2');
    expect(first.textContent).toContain('640 MB');
    expect(first.textContent).toContain('just now');
    expect(rows(fixture)[3]!.textContent).toContain('days ago');
    expect(el(fixture, 'tfoot').textContent).toContain('orphaned: 1.9 GB can be freed');
  });

  it('enables a checkbox only for orphaned rows; the others say why in a tooltip', async () => {
    const { fixture } = await setup();
    const boxes = all<HTMLInputElement>(fixture, 'tbody input[type=checkbox]');
    expect(boxes.map((b) => b.disabled)).toEqual([true, true, true, false, false]);
    expect(boxes[0]!.title).toContain('open');
    expect(boxes[3]!.hasAttribute('title')).toBe(false);
    expect(boxes[3]!.getAttribute('aria-label')).toBe('Select store-5e60b3f8');
    expect(button(fixture, 'stores-clean-selected').disabled).toBe(true);
    expect(button(fixture, 'stores-clean-all').disabled).toBe(false);
  });

  it('cleans up a ticked store after a confirmation that lists count and size, then shows the result and drops the row', async () => {
    const { fixture, backend, dialogs } = await setup();
    const clear = vi.spyOn(backend, 'clearStores');
    const boxes = all<HTMLInputElement>(fixture, 'tbody input[type=checkbox]');
    boxes[3]!.click();
    await settle(fixture);
    expect(button(fixture, 'stores-clean-selected').disabled).toBe(false);
    button(fixture, 'stores-clean-selected').click();
    await settle(fixture);
    const dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('Clean up project stores?');
    expect(String(dlg?.options.inputs?.['message'])).toContain('1 orphaned store (96 MB)');
    expect(String(dlg?.options.inputs?.['message'])).toContain('cannot be recovered');
    expect(clear).not.toHaveBeenCalled();
    dlg?.ref.close(true);
    await vi.waitFor(() => expect(clear).toHaveBeenCalledWith(['store-5e60b3f8']));
    await settle(fixture);
    expect(el(fixture, '[data-testid=stores-result]').textContent).toContain('Freed 96 MB.');
    expect(rows(fixture)).toHaveLength(4);
  });

  it('does nothing when the confirmation is dismissed', async () => {
    const { fixture, backend, dialogs } = await setup();
    const clear = vi.spyOn(backend, 'clearStores');
    button(fixture, 'stores-clean-all').click();
    await settle(fixture);
    const dlg = dialogs.stack().at(-1);
    expect(String(dlg?.options.inputs?.['message'])).toContain('2 orphaned stores (1.9 GB)');
    dlg?.ref.close();
    await settle(fixture);
    expect(clear).not.toHaveBeenCalled();
    expect(rows(fixture)).toHaveLength(5);
  });

  it('"Clean up all orphaned" removes both orphaned stores', async () => {
    const { fixture, dialogs } = await setup();
    button(fixture, 'stores-clean-all').click();
    await settle(fixture);
    dialogs.stack().at(-1)?.ref.close(true);
    await vi.waitFor(() => expect(rows(fixture)).toHaveLength(3));
    expect(button(fixture, 'stores-clean-all').disabled).toBe(true);
  });

  it('reports skipped stores with the reason', async () => {
    const { fixture, backend, dialogs } = await setup();
    vi.spyOn(backend, 'clearStores').mockResolvedValue({ freedBytes: 0, removed: [], skipped: [{ id: 'store-5e60b3f8', reason: 'a window has this store open' }], cancelled: true });
    button(fixture, 'stores-clean-all').click();
    await settle(fixture);
    dialogs.stack().at(-1)?.ref.close(true);
    await vi.waitFor(() => expect(el(fixture, '[data-testid=stores-result]')).toBeTruthy());
    const t = el(fixture, '[data-testid=stores-result]').textContent ?? '';
    expect(t).toContain('Freed 0 bytes.');
    expect(t).toContain('Kept store-5e60b3f8: a window has this store open.');
    expect(t).toContain('Stopped before every store was handled.');
  });

  it('shows a clean-up failure inline', async () => {
    const { fixture, backend, dialogs } = await setup();
    vi.spyOn(backend, 'clearStores').mockRejectedValue('the store folder is locked');
    button(fixture, 'stores-clean-all').click();
    await settle(fixture);
    dialogs.stack().at(-1)?.ref.close(true);
    await vi.waitFor(() => expect(el(fixture, '[data-testid=stores-error]').textContent).toContain('the store folder is locked'));
  });

  it('shows an empty state when there are no stores', async () => {
    const { fixture } = await setup();
    const backend = TestBed.inject(StorageBackend);
    vi.spyOn(backend, 'stores').mockResolvedValue({ folders: [], totalBytes: 0, orphanedBytes: 0, capped: false });
    await (fixture.componentInstance as unknown as { store: { loadStores(): Promise<void> } }).store.loadStores();
    await settle(fixture);
    expect(el(fixture, '[data-testid=stores-empty]').textContent).toContain('No stores on disk');
    expect(all(fixture, 'table')).toHaveLength(0);
  });

  it('shows a listing error with Retry', async () => {
    const { fixture } = await setup('?mockStoresFail=2');
    expect(el(fixture, '.banner.err').textContent).toContain('Could not list the project stores');
    el<HTMLButtonElement>(fixture, '.banner.err button').click();
    await settle(fixture);
    expect(rows(fixture)).toHaveLength(5);
  });

  it('binds the policy: shows the value, saves a valid one, refuses a bad one inline without calling the engine', async () => {
    const { fixture, backend } = await setup();
    const set = vi.spyOn(backend, 'setPolicy');
    const input = el<HTMLInputElement>(fixture, '[data-testid=stores-auto-days]');
    expect(input.value).toBe('0');
    typeInto(input, '30', 'change');
    await settle(fixture);
    expect(set).toHaveBeenCalledWith({ autoCleanDays: 30 });
    expect(input.value).toBe('30');
    typeInto(input, '400', 'change');
    await settle(fixture);
    expect(set).toHaveBeenCalledTimes(1);
    expect(el(fixture, '[data-testid=stores-policy-error]').textContent).toContain('At most 365');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    typeInto(input, '7', 'change');
    await settle(fixture);
    expect(set).toHaveBeenCalledWith({ autoCleanDays: 7 });
    expect(all(fixture, '[data-testid=stores-policy-error]')).toHaveLength(0);
  });

  it('shows the engine\'s refusal of the policy inline', async () => {
    const { fixture, backend } = await setup();
    vi.spyOn(backend, 'setPolicy').mockRejectedValue('automatic clean-up must be between 0 (off) and 365 days');
    typeInto(el<HTMLInputElement>(fixture, '[data-testid=stores-auto-days]'), '10', 'change');
    await vi.waitFor(() => expect(el(fixture, '[data-testid=stores-policy-error]').textContent).toContain('between 0'));
  });
});
