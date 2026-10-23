import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text, tinyBackend, tinyDataset } from '../../../testing/helpers';
import { at } from '../../../testing/must';
import { WorkspaceStore } from '../../core/workspace.store';
import { Sidebar } from '../../shell/sidebar';
import { ContextMenuService } from '../../ui/context-menu.service';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import type { MockDataset } from '../../core/mock/mock-data';
import { EncodingBackend } from './encoding-backend';
import { EncodingDialog } from './encoding-dialog';
import { MockEncodingBackend } from './mock/mock-encoding-backend';

function dataset(): MockDataset {
  const d = tinyDataset();
  return {
    ...d,
    sources: d.sources.map((s, i) => (i === 0 ? { ...s, encoding: { label: 'Shift_JIS', confidence: 57, origin: 'detected' as const } } : s)),
  };
}

async function setup() {
  const debrief = tinyBackend({ dataset });
  const backend = new MockEncodingBackend(debrief);
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  TestBed.configureTestingModule({ providers: [provideTinyBackend(debrief), { provide: EncodingBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }] });
  const workspace = TestBed.inject(WorkspaceStore);
  await workspace.init();
  return { backend, ref, workspace };
}

async function open(workspace: WorkspaceStore) {
  const fixture = TestBed.createComponent(EncodingDialog);
  fixture.componentRef.setInput('source', at(workspace.summary()!.sources, 0));
  await settle(fixture);
  await vi.waitFor(() => expect(all(fixture, 'label.choice').length).toBeGreaterThan(10));
  await settle(fixture);
  return fixture;
}

describe('Encoding…', () => {
  it('lists Automatic and the options, says the file is read again, and starts on Automatic for a guess', async () => {
    const { workspace } = await setup();
    const fixture = await open(workspace);
    expect(el<HTMLInputElement>(fixture, 'label.auto input').checked).toBe(true);
    expect(text(fixture)).toContain('Automatic (detect)');
    expect(text(fixture)).toContain('The file is read again');
    expect(el(fixture, '[data-testid=encoding-current]').textContent).toContain('Shift_JIS · 57% (detected)');
    expect(all(fixture, 'label.choice').length).toBe(22);
  });

  it('applies a choice: sets the encoding, closes, and the workspace follows the job', async () => {
    const { backend, ref, workspace } = await setup();
    const set = vi.spyOn(backend, 'setSourceEncoding');
    const fixture = await open(workspace);
    el<HTMLInputElement>(fixture, 'label[data-encoding="windows-1252"] input').click();
    el<HTMLButtonElement>(fixture, '[data-testid=encoding-apply]').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect(set).toHaveBeenCalledWith(0, 'windows-1252');
    await vi.waitFor(() => expect(workspace.summary()?.sources[0]?.encoding).toEqual({ label: 'windows-1252', confidence: 100, origin: 'manual' }), { timeout: 10_000 });

    // Reopened, the user's own choice is selected; Automatic detects again.
    const again = await open(workspace);
    expect(el<HTMLInputElement>(again, 'label[data-encoding="windows-1252"] input').checked).toBe(true);
    el<HTMLInputElement>(again, 'label.auto input').click();
    el<HTMLButtonElement>(again, '[data-testid=encoding-apply]').click();
    await vi.waitFor(() => expect(set).toHaveBeenLastCalledWith(0, null));
  });

  it('cancel closes without changing anything', async () => {
    const { backend, ref, workspace } = await setup();
    const set = vi.spyOn(backend, 'setSourceEncoding');
    const fixture = await open(workspace);
    el<HTMLButtonElement>(fixture, '[data-testid=encoding-cancel]').click();
    expect(ref.close).toHaveBeenCalledWith();
    expect(set).not.toHaveBeenCalled();
  });

  it('shows a failure inline and stays open', async () => {
    const { backend, ref, workspace } = await setup();
    vi.spyOn(backend, 'setSourceEncoding').mockRejectedValue('the engine said no');
    const fixture = await open(workspace);
    el<HTMLButtonElement>(fixture, '[data-testid=encoding-apply]').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(el(fixture, '[role=alert]').textContent).toContain('the engine said no');
    });
    expect(ref.close).not.toHaveBeenCalled();
    expect(el<HTMLButtonElement>(fixture, '[data-testid=encoding-apply]').disabled).toBe(false);
  });

  it('shows a failure to list the options', async () => {
    const { backend, workspace } = await setup();
    vi.spyOn(backend, 'options').mockRejectedValue('no list');
    const fixture = TestBed.createComponent(EncodingDialog);
    fixture.componentRef.setInput('source', at(workspace.summary()!.sources, 0));
    await settle(fixture);
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(text(fixture)).toContain('no list');
    });
  });

  it('is in the Explorer menu of a source with an encoding only, which the row badges', async () => {
    await setup();
    const fixture = await (async () => {
      const f = TestBed.createComponent(Sidebar);
      await settle(f);
      return f;
    })();
    const rows = all(fixture, '.src');
    expect(at(rows, 0).querySelector('[data-testid=encoding-badge]')?.textContent).toBe('Shift_JIS 57%');
    expect(at(rows, 0).querySelector('[data-testid=encoding-badge]')?.classList.contains('low')).toBe(true);
    expect(at(rows, 0).querySelector('[data-testid=encoding-badge]')?.getAttribute('title')).toContain('Guessed');
    expect(at(rows, 1).querySelector('[data-testid=encoding-badge]')).toBeNull();

    const menu = TestBed.inject(ContextMenuService);
    at(all(fixture, '[data-testid=source-actions]'), 1).click();
    expect(menu.menu()!.items.some((i) => i.label === 'Encoding…')).toBe(false);
    at(all(fixture, '[data-testid=source-actions]'), 0).click();
    menu.menu()!.items.find((i) => i.label === 'Encoding…')!.run();
    const dialogs = TestBed.inject(DialogService);
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.component).toBe(EncodingDialog));
  });
});
