import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text, tinyBackend } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { Sidebar } from '../../shell/sidebar';
import { ContextMenuService } from '../../ui/context-menu.service';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { at } from '../../../testing/must';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParsersBackend } from './parsers-backend';
import { initialChoice, ParseWithDialog } from './parse-with-dialog';
import type { ParserLibraryDto } from '../../core/api.types';

async function setup() {
  const imt = tinyBackend();
  const backend = new MockParsersBackend(imt);
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  TestBed.configureTestingModule({ providers: [provideTinyBackend(imt), { provide: ParsersBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }] });
  const workspace = TestBed.inject(WorkspaceStore);
  await workspace.init();
  return { backend, ref, workspace };
}

describe('Parse with…', () => {
  it('lists the library (enabled first, disabled marked) with the current parser marked, and re-reads the source', async () => {
    const { backend, ref, workspace } = await setup();
    await backend.setEnabled('nlog', false);
    const fixture = TestBed.createComponent(ParseWithDialog);
    fixture.componentRef.setInput('source', at(workspace.summary()!.sources, 1));
    await settle(fixture);
    await vi.waitFor(() => expect(all(fixture, 'label.choice').length).toBeGreaterThan(5));
    await settle(fixture);
    const current = el(fixture, 'label[data-parser="serilog-text"]');
    // Matched automatically: "Automatic" stays selected, so confirming without a change does not pin it.
    expect(current.querySelector<HTMLInputElement>('input')!.checked).toBe(false);
    expect(el<HTMLInputElement>(fixture, 'label.auto input').checked).toBe(true);
    expect(current.textContent).toContain('read with it now');
    const labels = all(fixture, 'label.choice');
    expect(labels.at(-1)?.getAttribute('data-parser')).toBe('nlog');
    expect(labels.at(-1)?.textContent).toContain('disabled');
    expect(text(fixture)).toContain('Automatic');

    const assign = vi.spyOn(backend, 'assign');
    el<HTMLInputElement>(fixture, 'label[data-parser="nlog"] input').click();
    el<HTMLButtonElement>(fixture, '[data-testid=parse-with-apply]').click();
    expect(ref.close).toHaveBeenCalledWith(true);
    await vi.waitFor(() => expect(workspace.summary()?.sources[1]?.parser).toBe('nlog'), { timeout: 10_000 });
    expect(assign).toHaveBeenCalledWith(1, 'nlog');

    el<HTMLInputElement>(fixture, 'label.auto input').click();
    el<HTMLButtonElement>(fixture, '[data-testid=parse-with-apply]').click();
    await vi.waitFor(() => expect(assign).toHaveBeenLastCalledWith(1, null));
  });

  it('confirming without a change keeps "Automatic"; a parser the user chose is selected again', async () => {
    const { backend, workspace } = await setup();
    const assign = vi.spyOn(backend, 'assign');
    const open = async () => {
      const fixture = TestBed.createComponent(ParseWithDialog);
      fixture.componentRef.setInput('source', at(workspace.summary()!.sources, 1));
      await settle(fixture);
      await vi.waitFor(() => expect(all(fixture, 'label.choice').length).toBeGreaterThan(5));
      await settle(fixture);
      return fixture;
    };
    let fixture = await open();
    el<HTMLButtonElement>(fixture, '[data-testid=parse-with-apply]').click();
    expect(assign).toHaveBeenLastCalledWith(1, null);
    await vi.waitFor(() => expect(workspace.summary()?.sources[1]?.parser).toBe('serilog-text'), { timeout: 10_000 });

    fixture = await open();
    el<HTMLInputElement>(fixture, 'label[data-parser="nlog"] input').click();
    el<HTMLButtonElement>(fixture, '[data-testid=parse-with-apply]').click();
    await vi.waitFor(() => expect(workspace.summary()?.sources[1]?.parser).toBe('nlog'), { timeout: 10_000 });
    fixture = await open();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(el<HTMLInputElement>(fixture, 'label[data-parser="nlog"] input').checked).toBe(true);
    });
    el<HTMLButtonElement>(fixture, '[data-testid=parse-with-apply]').click();
    expect(assign).toHaveBeenLastCalledWith(1, 'nlog');
  });

  it('selects at first what the user chose before, else Automatic', () => {
    const lib = (chosen: boolean) =>
      ({ parsers: [{ id: 'nlog', sources: [{ id: 3, name: 'a.log', chosen }] }], broken: [], stale: 0 }) as unknown as ParserLibraryDto;
    expect(initialChoice(null, 3)).toBe('');
    expect(initialChoice(lib(false), 3)).toBe('');
    expect(initialChoice(lib(true), 3)).toBe('nlog');
    expect(initialChoice(lib(true), 4)).toBe('');
  });

  it('starts on "Automatic" for a source no parser read', async () => {
    const { workspace } = await setup();
    const fixture = TestBed.createComponent(ParseWithDialog);
    fixture.componentRef.setInput('source', { ...at(workspace.summary()!.sources, 0), parser: null });
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'label.auto input').checked).toBe(true);
  });

  it('is in the Explorer menu of a source, whose tooltip names its parser', async () => {
    await setup();
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    expect(at(all(fixture, '.src'), 1).getAttribute('title')).toContain('Read with the parser serilog-text');
    const menu = TestBed.inject(ContextMenuService);
    at(all(fixture, '[data-testid=source-actions]'), 1).click();
    const item = menu.menu()!.items.find((i) => i.label === 'Parse with…')!;
    item.run();
    const dialogs = TestBed.inject(DialogService);
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.component).toBe(ParseWithDialog));
    expect(dialogs.stack().at(-1)?.options.title).toBe('Parse app.log with');
  });
});
