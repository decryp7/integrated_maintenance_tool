import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { DIALOG_REF } from '../../ui/dialog.service';
import { MockPasteBackend } from './mock/mock-paste-backend';
import { PasteBackend } from './paste-backend';
import { PasteDialog } from './paste-dialog';

async function setup(clipboard: string | Error = 'clip text') {
  const imt = tinyBackend();
  const backend = new MockPasteBackend(imt);
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { readText: () => (clipboard instanceof Error ? Promise.reject(clipboard) : Promise.resolve(clipboard)) },
  });
  TestBed.configureTestingModule({ providers: [provideTinyBackend(imt), { provide: PasteBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }] });
  const workspace = TestBed.inject(WorkspaceStore);
  await workspace.init();
  return { backend, ref, workspace };
}

const area = (f: Parameters<typeof el>[0]) => el<HTMLTextAreaElement>(f, '[data-testid=paste-text]');
const addButton = (f: Parameters<typeof el>[0]) => el<HTMLButtonElement>(f, '[data-testid=paste-add]');

describe('Paste as log dialog', () => {
  it('starts with the default name and the clipboard text, and shows the byte counter', async () => {
    await setup('héllo');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, '[data-testid=paste-name]').value).toBe('Pasted log');
    expect(area(fixture).value).toBe('héllo');
    expect(el(fixture, '[data-testid=paste-count]').textContent).toContain('6 bytes of 8 MB');
    expect(addButton(fixture).disabled).toBe(false);
  });

  it('works when the clipboard cannot be read: empty box, Add disabled', async () => {
    await setup(new Error('denied'));
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    expect(area(fixture).value).toBe('');
    expect(addButton(fixture).disabled).toBe(true);
  });

  it('takes the dropped text instead of the clipboard', async () => {
    await setup('clip');
    const fixture = TestBed.createComponent(PasteDialog);
    fixture.componentRef.setInput('text', 'dropped');
    await settle(fixture);
    expect(area(fixture).value).toBe('dropped');
  });

  it('refuses text over 8 MiB early', async () => {
    const { backend } = await setup('');
    const add = vi.spyOn(backend, 'pasteLog');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    typeInto(area(fixture), 'x'.repeat(8 * 1024 * 1024 + 1));
    await settle(fixture);
    expect(addButton(fixture).disabled).toBe(true);
    expect(text(fixture)).toContain('Save it as a file and open that instead');
    expect(all(fixture, '[role=alert]').length).toBe(1);
    addButton(fixture).click();
    expect(add).not.toHaveBeenCalled();
  });

  it('adds: sends the text and the name, applies the workspace and closes', async () => {
    const { backend, ref, workspace } = await setup('');
    const add = vi.spyOn(backend, 'pasteLog');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, '[data-testid=paste-name]'), '  web01 errors ');
    typeInto(area(fixture), 'ERROR boom');
    await settle(fixture);
    addButton(fixture).click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect(add).toHaveBeenCalledWith('ERROR boom', 'web01 errors');
    expect(workspace.summary()?.sources.some((s) => s.name === 'web01 errors.log')).toBe(true);
  });

  it('sends null for an empty name', async () => {
    const { backend } = await setup('x');
    const add = vi.spyOn(backend, 'pasteLog');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, '[data-testid=paste-name]'), '');
    addButton(fixture).click();
    await vi.waitFor(() => expect(add).toHaveBeenCalledWith('x', null));
  });

  it('shows the engine error inline and stays open', async () => {
    const { backend, ref } = await setup('x');
    vi.spyOn(backend, 'pasteLog').mockRejectedValue('the project is read-only');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    addButton(fixture).click();
    await vi.waitFor(() => expect(el(fixture, '[data-testid=paste-error]').textContent).toContain('the project is read-only'));
    expect(ref.close).not.toHaveBeenCalled();
    expect(addButton(fixture).disabled).toBe(false);
  });

  it('cancels', async () => {
    const { ref } = await setup('x');
    const fixture = TestBed.createComponent(PasteDialog);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=paste-cancel]').click();
    expect(ref.close).toHaveBeenCalledWith();
  });
});
