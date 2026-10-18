import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, tinyBackend, text, typeInto } from '../../../testing/helpers';
import { MarkDialog } from '../../context/mark-dialog';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { WorkspaceStore } from '../../core/workspace.store';
import { parseProfile } from '../../profile/toml';
import { RegexApi } from './regex-api';
import { RegexAssist, splitByBytes } from './regex-assist';
import { fieldTypeFor, InferDialog, renameGroups } from './infer-dialog';
import { qlQuote } from '../../analysis/ql-text';
import { InferredRegex } from '../../core/api.types';
import { at } from '../../../testing/must';

const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };

describe('RegexApi', () => {
  it('asks the engine through the backend', async () => {
    const backend = tinyBackend();
    const result: InferredRegex = { pattern: 'p', groups: [], matches: [true], explanation: [] };
    const infer = vi.spyOn(backend, 'inferRegex').mockResolvedValue(result);
    const explain = vi.spyOn(backend, 'explainRegex').mockResolvedValue([{ depth: 0, text: 'x', start: 0, end: 1 }]);
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const api = TestBed.inject(RegexApi);
    expect(await api.infer(['a'])).toBe(result);
    expect(infer).toHaveBeenCalledWith(['a']);
    expect(await api.explain('x')).toHaveLength(1);
    expect(explain).toHaveBeenCalledWith('x');
  });
  it('passes engine errors through', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'explainRegex').mockRejectedValue('regex parse error');
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    await expect(TestBed.inject(RegexApi).explain('(')).rejects.toBe('regex parse error');
  });
});

describe('splitByBytes', () => {
  it('splits by byte offsets and clamps', () => {
    expect(splitByBytes('aéb', 1, 3)).toEqual(['a', 'é', 'b']);
    expect(splitByBytes('abc', -5, 99)).toEqual(['', 'abc', '']);
    expect(splitByBytes('abc', 2, 1)).toEqual(['ab', '', 'c']);
  });
});

describe('RegexAssist', () => {
  async function mount(inputs: Record<string, unknown>, backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const f = TestBed.createComponent(RegexAssist);
    for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
    await settle(f);
    return f;
  }
  it('explains on demand, highlights the hovered part and follows edits', async () => {
    const f = await mount({ pattern: 'ab\\d+' });
    expect(f.nativeElement.querySelector('.panel')).toBeNull();
    buttonByText(f, 'Explain').click();
    await settle(f, 4);
    expect(f.nativeElement.querySelector('button').getAttribute('aria-expanded')).toBe('true');
    const items = all(f, 'li');
    expect(items.map((i) => i.textContent)).toEqual(['Literal "ab"', 'One or more digits']);
    at(items, 1).dispatchEvent(new Event('mouseenter'));
    await settle(f);
    expect(el(f, 'mark').textContent).toBe('\\d+');
    at(items, 1).dispatchEvent(new Event('mouseleave'));
    at(items, 0).dispatchEvent(new Event('focus'));
    await settle(f);
    expect(el(f, 'mark').textContent).toBe('ab');
    at(items, 0).dispatchEvent(new Event('blur'));
    f.componentRef.setInput('pattern', 'zz');
    await settle(f, 4);
    expect(all(f, 'li').map((i) => i.textContent)).toEqual(['Literal "zz"']);
    buttonByText(f, 'Explain').click();
    await settle(f);
    expect(f.nativeElement.querySelector('.panel')).toBeNull();
  });
  it('shows engine errors and the empty outline, and uses a preset outline without asking', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'explainRegex').mockRejectedValue(new Error('invalid regex'));
    const f = await mount({ pattern: '(' }, backend);
    buttonByText(f, 'Explain').click();
    await settle(f, 4);
    expect(text(f)).toContain('invalid regex');
    TestBed.resetTestingModule();
    const g = await mount({ pattern: 'a', preset: [] });
    buttonByText(g, 'Explain').click();
    await settle(g);
    expect(text(g)).toContain('Nothing to explain');
    await (g.componentInstance as unknown as { refresh(): Promise<void> }).refresh();
  });
});

describe('InferDialog helpers', () => {
  it('quotes for the query language, renames groups and maps kinds to field types', () => {
    expect(qlQuote('a"b\\d')).toBe('"a\\"b\\\\d"');
    expect(renameGroups('(?P<n1>\\d+)-(?<n2>x)(?P<n3>y)', { n1: 'Id', n2: 'n2' })).toBe('(?P<Id>\\d+)-(?<n2>x)(?P<n3>y)');
    expect(fieldTypeFor('number')).toBe('integer');
    expect(fieldTypeFor('decimal')).toBe('number');
    expect(fieldTypeFor('guid')).toBe('string');
  });
});

describe('InferDialog', () => {
  async function mount(lines: string[], backend = tinyBackend()) {
    ref.close.mockClear();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: ref }] });
    const f = TestBed.createComponent(InferDialog);
    f.componentRef.setInput('lines', lines);
    await settle(f, 4);
    await new Promise((r) => setTimeout(r, 20));
    await settle(f, 4);
    return { f, backend };
  }
  const lines = ['Slow request /a took 1500 ms', 'Slow request /b took 20 ms', 'unrelated'];

  it('shows the pattern, groups, per-line results and the explanation', async () => {
    const { f } = await mount(lines);
    expect(el<HTMLInputElement>(f, 'input.mono').value).toBe('Slow request /a took (?P<n1>\\d+) ms');
    expect(all(f, '.groups tbody tr')).toHaveLength(1);
    expect(all(f, '.lines li').map((l) => l.textContent!.trim().charAt(0))).toEqual(['✗', '✗', '✗'].map((_, i) => (i === 0 ? '✓' : '✗')));
    buttonByText(f, 'Explain').click();
    await settle(f);
    expect(all(f, 'li[tabindex]').length).toBeGreaterThan(2);
  });

  it('re-infers when the lines are edited and reports an empty input', async () => {
    const { f } = await mount(lines);
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), 'x 5\ny 6');
    await new Promise((r) => setTimeout(r, 350));
    await settle(f, 4);
    expect(el<HTMLInputElement>(f, 'input.mono').value).toBe('x (?P<n1>\\d+)');
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), '');
    await new Promise((r) => setTimeout(r, 350));
    await settle(f, 4);
    expect(el(f, '[role=alert]').textContent).toContain('Add at least one line');
  });

  it('reports inference failures', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'inferRegex').mockRejectedValue(new Error('too many lines'));
    const { f } = await mount(['a'], backend);
    expect(el(f, '[role=alert]').textContent).toContain('too many lines');
  });

  it('opens the marker editor with the regex as a matches condition', async () => {
    const { f } = await mount(lines);
    rename(f, 'Ms');
    buttonByText(f, 'Highlight these lines').click();
    expect(ref.close).toHaveBeenCalled();
    const d = at(TestBed.inject(DialogService).stack(), 0);
    expect(d.component).toBe(MarkDialog);
    expect(d.options.inputs?.['initial']).toEqual([{ field: '@message', op: 'matches', value: 'Slow request /a took (?P<Ms>\\d+) ms' }]);
  });

  it('filters the workspace by the pattern', async () => {
    const { f } = await mount(lines);
    buttonByText(f, 'Filter').click();
    expect(TestBed.inject(WorkspaceStore).query().ql).toBe('message ~ "Slow request /a took (?P<n1>\\\\d+) ms"');
    expect(ref.close).toHaveBeenCalled();
  });

  it('extracts the named groups as typed fields', async () => {
    const { f, backend } = await mount(lines);
    rename(f, 'ElapsedMs');
    buttonByText(f, 'Extract fields…').click();
    await settle(f, 8);
    expect(ref.close).toHaveBeenCalledWith(true);
    const x = (parseProfile((await backend.getProfile()).toml)['extract'] as { name: string; regex: string; fields: Record<string, { type: string }> }[]).at(-1)!;
    expect(x.name).toBe('extract-elapsedms');
    expect(x.regex).toContain('(?P<ElapsedMs>');
    expect(x.fields).toEqual({ ElapsedMs: { type: 'integer' } });
  });

  it('blocks extraction with an invalid group name and shows save failures', async () => {
    const { f, backend } = await mount(lines);
    rename(f, '1bad');
    await settle(f);
    expect(buttonByText(f, 'Extract fields…').disabled).toBe(true);
    expect(el(f, '[role=alert]').textContent).toContain('1bad');
    rename(f, 'good');
    await settle(f);
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'bad extractor', line: 3 });
    buttonByText(f, 'Extract fields…').click();
    await settle(f, 6);
    expect(el(f, '[role=alert]').textContent).toContain('bad extractor (profile line 3)');
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'again', line: null });
    buttonByText(f, 'Extract fields…').click();
    await settle(f, 6);
    expect(el(f, '[role=alert]').textContent).toContain('again');
  });

  it('says so when the lines are identical', async () => {
    const { f } = await mount(['same', 'same']);
    expect(text(f)).toContain('No variable parts');
    expect(buttonByText(f, 'Extract fields…').disabled).toBe(true);
    buttonByText(f, 'Close').click();
    expect(ref.close).toHaveBeenCalled();
  });
});

function rename(f: Parameters<typeof el>[0], name: string) {
  typeInto(el<HTMLInputElement>(f, '.groups input'), name);
}
