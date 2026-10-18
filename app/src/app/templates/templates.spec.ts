import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { ParsePreview } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { mockPreviewParse } from '../core/mock/mock-parse';
import { MOCK_TEMPLATES } from '../core/mock/mock-templates';
import { UxBackend } from '../core/ux-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { UndoStack } from '../undo/undo-stack';
import { bestTemplate, groupTemplates, ratioLabel, readRatio } from './templates';
import { TemplatesGallery } from './templates-gallery';
import { TemplatesService } from './templates.service';
import { at } from '../../testing/must';

const preview = (patch: Partial<ParsePreview>): ParsePreview => ({ importer: 'text', records: [], total: 0, skipped: 0, dropped: 0, error: null, ...patch });

describe('template helpers', () => {
  it('groups by category in order', () => {
    const t = (id: string, category: string) => ({ ...at(MOCK_TEMPLATES, 0), id, category });
    expect(groupTemplates([t('a', 'X'), t('b', 'Y'), t('c', 'X')]).map((g) => [g.category, g.templates.map((x) => x.id)])).toEqual([
      ['X', ['a', 'c']],
      ['Y', ['b']],
    ]);
  });

  it('rates how much of a sample a template reads and picks the best', () => {
    expect(readRatio(preview({ total: 3, skipped: 1 }))).toBe(0.75);
    expect(readRatio(preview({ total: 0 }))).toBe(0);
    expect(readRatio(preview({ total: 3, error: 'bad' }))).toBe(0);
    expect(bestTemplate(new Map([['a', 0.2], ['b', 0.9], ['c', 0.9]]))).toBe('b');
    expect(bestTemplate(new Map([['a', 0]]))).toBeNull();
    expect(ratioLabel(0)).toBe('reads nothing');
    expect(ratioLabel(0.954)).toBe('reads 95%');
  });

  it('every mock template is valid and reads its own sample (the mock mirrors the engine contract)', () => {
    expect(MOCK_TEMPLATES.length).toBeGreaterThanOrEqual(13);
    for (const t of MOCK_TEMPLATES.filter((x) => x.sampleText)) {
      const p = mockPreviewParse({ toml: t.toml, sample: t.sampleText, fileName: t.sampleName, limit: 10 });
      expect(p.error, t.id).toBeNull();
      expect(p.total, t.id).toBeGreaterThan(0);
      expect(t.description.length, t.id).toBeGreaterThan(20);
    }
  });
});

describe('TemplatesGallery', () => {
  async function render(inputs: { path?: string | null; name?: string | null } = {}, backend: MockBackend = tinyBackend()) {
    const close = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: { close, closed: new Promise(() => undefined) } }] });
    const fixture = TestBed.createComponent(TemplatesGallery);
    for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
    await settle(fixture, 8);
    return { fixture, close, backend };
  }

  it('lists templates by category, previews the selected one on its sample and switches', async () => {
    const { fixture } = await render();
    expect(text(fixture)).toContain('The preview uses a short sample');
    expect(all(fixture, 'nav h3').map((h) => h.textContent)).toContain('Web servers');
    expect(el(fixture, '.tpl.on').getAttribute('aria-pressed')).toBe('true');
    expect(el(fixture, '[data-testid=template-preview-status]').textContent).toContain('Reads');
    expect(all(fixture, 'tbody tr').length).toBeGreaterThan(0);
    el<HTMLButtonElement>(fixture, '[data-template=nlog]').click();
    await settle(fixture, 6);
    expect(el(fixture, '.detail .title').textContent).toContain('NLog');
    expect(text(fixture)).toContain('its sample');
  });

  it('rates the templates against the user file and selects the best one', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'readFileHead').mockResolvedValue('noise\n2026-10-14 10:41:00.1204|INFO|A|started\n2026-10-14 10:41:01.0000|ERROR|A|failed\n');
    const { fixture } = await render({ path: '/logs/system/export.reg', name: 'export.reg' }, backend);
    expect(text(fixture)).toContain('reads export.reg');
    expect(el(fixture, '.tpl.on').getAttribute('data-template')).toBe('nlog');
    expect(el(fixture, '.tpl.on .badge').textContent).toContain('67%');
    expect(text(fixture)).toContain('the start of export.reg');
    expect(text(fixture)).toContain('1 line not understood');
  });

  it('falls back to the samples when the file cannot be read, with Retry', async () => {
    const backend = tinyBackend();
    const head = vi.spyOn(backend, 'readFileHead').mockRejectedValueOnce('locked');
    const { fixture } = await render({ path: '/x.log', name: 'x.log' }, backend);
    expect(text(fixture)).toContain('Could not read x.log: locked');
    head.mockResolvedValue('nothing parses here');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 8);
    expect(text(fixture)).toContain('reads nothing');
    expect(text(fixture)).toContain('reads no events from the start of x.log');
  });

  it('explains that a binary format (EVTX) has no text sample to preview', async () => {
    const { fixture } = await render();
    el<HTMLButtonElement>(fixture, '[data-template=windows-event]').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('reads a binary format, so there is no text sample to preview');
  });

  it('shows preview failures and engine errors', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'previewParse').mockRejectedValueOnce('engine down');
    const { fixture } = await render({}, backend);
    expect(text(fixture)).toContain('Preview failed: engine down');
    spy.mockResolvedValueOnce(preview({ error: 'bad regex' }));
    el<HTMLButtonElement>(fixture, '[data-template=csv]').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('bad regex');
  });

  it('applies a template through validate + save, undoably, and closes', async () => {
    const { fixture, close, backend } = await render();
    const before = (await backend.getProfile()).toml;
    el<HTMLButtonElement>(fixture, '[data-template=csv]').click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Use this').click();
    await settle(fixture, 8);
    expect(close).toHaveBeenCalledWith(true);
    expect((await backend.getProfile()).toml).toContain('CSV with a timestamp column');
    expect(TestBed.inject(WorkspaceStore).notice()).toContain('Applied the CSV with a timestamp column template');
    const undo = TestBed.inject(UndoStack);
    expect(undo.undoLabel()).toBe('profile template');
    await undo.undo();
    expect((await backend.getProfile()).toml).toBe(before);
  });

  it('keeps the dialog open with the reason when the engine refuses the template', async () => {
    const { fixture, close, backend } = await render();
    const validate = vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'unknown key', line: 3 });
    buttonByText(fixture, 'Use this').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('The profile was not applied: line 3: unknown key');
    validate.mockResolvedValueOnce({ ok: false, error: null, line: null });
    buttonByText(fixture, 'Use this').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('the profile is not valid');
    vi.spyOn(backend, 'saveProfile').mockRejectedValueOnce('disk full');
    buttonByText(fixture, 'Use this').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('disk full');
    expect(close).not.toHaveBeenCalled();
  });

  it('reports a failure to load the templates with Retry, and an empty catalogue', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: DIALOG_REF, useValue: { close: vi.fn(), closed: new Promise(() => undefined) } }] });
    vi.spyOn(TestBed.inject(UxBackend), 'profileTemplates').mockRejectedValueOnce('nope').mockResolvedValueOnce([]);
    const fixture = TestBed.createComponent(TemplatesGallery);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Could not load the templates: nope');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No templates are installed');
  });
});

describe('TemplatesService', () => {
  it('opens the gallery for a file and reports whether a template was applied', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const dialogs = TestBed.inject(DialogService);
    const done = TestBed.inject(TemplatesService).open({ path: '/a.log', name: 'a.log' });
    await vi.waitFor(() => expect(dialogs.stack().length).toBe(1));
    const top = at(dialogs.stack(), 0);
    expect(top.component).toBe(TemplatesGallery);
    expect(top.options.inputs).toEqual({ path: '/a.log', name: 'a.log' });
    top.ref.close(true);
    expect(await done).toBe(true);
    const again = TestBed.inject(TemplatesService).open();
    await vi.waitFor(() => expect(dialogs.stack().length).toBe(1));
    expect(dialogs.stack()[0]?.options.inputs).toEqual({ path: null, name: null });
    dialogs.dismissTop();
    expect(await again).toBe(false);
  });
});
