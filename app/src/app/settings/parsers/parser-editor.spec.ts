import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../../testing/helpers';
import { ParserDocDto, ParsePreviewRequest } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParserDialogs } from './parser-dialogs';
import { ParserDraft } from './parser-draft';
import { ParserEditor } from './parser-editor';
import { ParsersBackend } from './parsers-backend';
import { BLANK_PARSER } from './parsers.store';

const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };

async function mount(doc?: (b: MockParsersBackend) => Promise<ParserDocDto>, inputs: { isNew?: boolean; readSources?: number[] } = {}) {
  ref.close.mockClear();
  const debrief = tinyBackend();
  const backend = new MockParsersBackend(debrief);
  const requests: ParsePreviewRequest[] = [];
  const preview = debrief.previewParse.bind(debrief);
  vi.spyOn(debrief, 'previewParse').mockImplementation((r) => (requests.push(r), preview(r)));
  TestBed.configureTestingModule({ providers: [provideTinyBackend(debrief), { provide: ParsersBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }] });
  await TestBed.inject(WorkspaceStore).init();
  const opened = doc ? await doc(backend) : await backend.get('serilog-text');
  const fixture = TestBed.createComponent(ParserEditor);
  fixture.componentRef.setInput('doc', opened);
  if (inputs.isNew !== undefined) fixture.componentRef.setInput('isNew', inputs.isNew);
  if (inputs.readSources) fixture.componentRef.setInput('readSources', inputs.readSources);
  await settle(fixture);
  return { fixture, backend, debrief, requests };
}

/** Lets the debounced validation and preview run. */
async function idle(fixture: Awaited<ReturnType<typeof mount>>['fixture']) {
  await new Promise((r) => setTimeout(r, 350));
  await settle(fixture);
}

const area = (fixture: Awaited<ReturnType<typeof mount>>['fixture']) => el<HTMLTextAreaElement>(fixture, 'textarea');

describe('ParserEditor', () => {
  it('validates the TOML, previews it against a file of the window and says a built-in saves as an override', async () => {
    const { fixture, requests } = await mount(undefined, { readSources: [1] });
    await idle(fixture);
    expect(el(fixture, '[data-testid=parser-builtin-note]').textContent).toContain('Saving keeps your version as an override');
    expect(area(fixture).getAttribute('aria-label')).toBe('Parser TOML');
    expect(el(fixture, '[data-testid=parser-validation]').textContent).toContain('Valid: Serilog text 1.0.0 (serilog-text)');
    expect(el<HTMLSelectElement>(fixture, '[data-testid=parser-preview-file]').value).toBe('1');
    expect(requests.at(-1)).toMatchObject({ fileName: 'app.log', limit: 50 });
    expect(requests.at(-1)?.toml).toContain('id = "serilog-text"');
    expect(el(fixture, '[data-testid=parser-preview-summary]').textContent).toContain('4 events read');
    expect(text(fixture)).toContain('Payment capture failed');
    // Unchanged: nothing to save yet.
    expect(buttonByText(fixture, 'Save as override').disabled).toBe(true);
  });

  it('pins a validation error to its line and column and keeps Save disabled', async () => {
    const { fixture } = await mount();
    typeInto(area(fixture), 'schema = 1\nid = "Bad Id"\nname = "x"\nversion = "1.0.0"\n');
    await idle(fixture);
    expect(text(fixture)).toContain('Line 2, column 1');
    expect(text(fixture)).toContain('invalid id "Bad Id"');
    expect(buttonByText(fixture, 'Save as override').disabled).toBe(true);
    expect(el(fixture, '.gutter .err').textContent?.trim()).toBe('2');
  });

  it('saves an override of a built-in and closes with the library', async () => {
    const { fixture, backend } = await mount();
    const save = vi.spyOn(backend, 'save');
    typeInto(area(fixture), (await backend.get('serilog-text')).toml.replace('version = "1.0.0"', 'version = "1.0.1"'));
    await idle(fixture);
    buttonByText(fixture, 'Save as override').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(expect.objectContaining({ parsers: expect.any(Array) })));
    expect(save).toHaveBeenCalledWith(expect.stringContaining('1.0.1'), { previousId: 'serilog-text' });
    expect((await backend.get('serilog-text')).origin).toBe('override');
  });

  it('asks before replacing another parser with the same id, and shows save errors', async () => {
    const { fixture, backend } = await mount(async (b) => {
      await b.save('schema = 1\nid = "taken"\nname = "Taken"\nversion = "1.0.0"\n');
      return { id: '', origin: 'user', toml: BLANK_PARSER };
    }, { isNew: true });
    const dialogs = TestBed.inject(DialogService);
    await idle(fixture);
    expect(buttonByText(fixture, 'Save').disabled).toBe(false);
    typeInto(area(fixture), 'schema = 1\nid = "taken"\nname = "Mine"\nversion = "1.0.0"\n');
    await idle(fixture);
    buttonByText(fixture, 'Save').click();
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.options.title).toBe('Replace parser?'));
    dialogs.stack().at(-1)!.ref.close();
    await vi.waitFor(() => expect(text(fixture)).toContain('already in the library'));
    expect(ref.close).not.toHaveBeenCalled();
    buttonByText(fixture, 'Save').click();
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    dialogs.stack().at(-1)!.ref.close(true);
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalled());
    expect((await backend.get('taken')).toml).toContain('name = "Mine"');
  });

  it('switches the preview file, retries a file that cannot be read and shows preview failures', async () => {
    const { fixture, debrief } = await mount();
    await idle(fixture);
    const head = vi.spyOn(debrief, 'readFileHead').mockRejectedValueOnce(new Error('locked'));
    typeInto(el<HTMLSelectElement>(fixture, '[data-testid=parser-preview-file]'), '0', 'change');
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('locked');
    buttonByText(fixture, 'Retry').click();
    await idle(fixture);
    expect(head).toHaveBeenLastCalledWith('/logs/web01/app.clef', 32768);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    vi.spyOn(debrief, 'previewParse').mockRejectedValueOnce(new Error('preview failed'));
    typeInto(area(fixture), (await TestBed.inject(ParsersBackend).get('nlog')).toml);
    await idle(fixture);
    expect(el(fixture, '[data-testid=parser-preview-summary]').textContent).toContain('preview failed');
  });

  it('says how to preview without files, and the draft reports an unreachable engine', async () => {
    const { fixture, backend } = await mount();
    await TestBed.inject(WorkspaceStore).clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('Open a log in this window to preview');
    const draft = fixture.debugElement.injector.get(ParserDraft);
    vi.spyOn(backend, 'validate').mockRejectedValueOnce(new Error('engine gone'));
    draft.setText('x');
    await idle(fixture);
    expect(draft.problem()).toEqual({ message: 'engine gone', line: null, column: null });
    draft.ngOnDestroy();
  });

  it('is opened by ParserDialogs, which also asks the questions', async () => {
    await mount();
    const dialogs = TestBed.inject(DialogService);
    const parserDialogs = TestBed.inject(ParserDialogs);
    const edit = parserDialogs.edit({ id: 'nlog', origin: 'builtin', toml: '' }, { readSources: [1] });
    expect(dialogs.stack().at(-1)?.options).toMatchObject({ title: 'Edit parser nlog', inputs: { isNew: false, readSources: [1] } });
    dialogs.stack().at(-1)!.ref.close();
    expect(await edit).toBeUndefined();
    void parserDialogs.edit({ id: '', origin: 'user', toml: '' }, { isNew: true });
    expect(dialogs.stack().at(-1)?.options.title).toBe('New parser');
    dialogs.stack().at(-1)!.ref.close();

    const entry = { id: 'mine', name: 'Mine', version: '1.0.0', description: '', category: 'Custom', kind: 'text', priority: 0, origin: 'user' as const, enabled: true, hash: '', sources: [] };
    const remove = parserDialogs.confirmRemove(entry);
    expect(String(dialogs.stack().at(-1)?.options.inputs?.['message'])).toContain('Remove the parser Mine?');
    dialogs.stack().at(-1)!.ref.close(true);
    expect(await remove).toBe(true);
    const removeOverride = parserDialogs.confirmRemove({ ...entry, origin: 'override' });
    expect(String(dialogs.stack().at(-1)?.options.inputs?.['message'])).toContain('built-in parser is used again');
    dialogs.stack().at(-1)!.ref.close();
    expect(await removeOverride).toBe(false);

    const check = { toml: '', id: 'x', name: 'X', version: '2.0.0', relation: 'newer' as const, existing: null };
    const update = parserDialogs.confirmImport(check);
    expect(dialogs.stack().at(-1)?.options.title).toBe('Update parser');
    dialogs.stack().at(-1)!.ref.close(true);
    expect(await update).toBe(true);
    void parserDialogs.confirmImport({ ...check, relation: 'same' });
    expect(dialogs.stack().at(-1)?.options.title).toBe('Replace parser');
  });
});
