import { TestBed } from '@angular/core/testing';
import { Type } from '@angular/core';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { ContextMenuHost } from '../ui/context-menu-host';
import { ContextMenuService } from '../ui/context-menu.service';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { DropDialog } from './drop-dialog';
import { ExtractDialog } from './extract-dialog';
import { InferDialog } from '../markers/regex/infer-dialog';
import { MarkDialog } from './mark-dialog';
import { applyMask, MaskDialog } from './mask-dialog';
import { MatchCounter } from './match-count';
import { copyTextFor, groupMenu, RecordActions } from './record-actions';
import { GridSelection } from '../grid/grid-selection';
import { selectionWithin } from './selection';
import { at } from '../../testing/must';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };

async function mount<T>(cmp: Type<T>, inputs: Record<string, unknown>, backend = tinyBackend()) {
  ref.close.mockClear();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: ref }] });
  const fixture = TestBed.createComponent(cmp);
  for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
  await settle(fixture);
  const store = TestBed.inject(WorkspaceStore);
  return { fixture, store, backend };
}

async function record(match: (r: RecordView) => boolean): Promise<RecordView> {
  const page = await TestBed.inject(DebriefBackend).queryPage({
    query: { from: null, to: null, levels: [], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [] },
    offset: 0,
    limit: 100,
    descending: false,
  });
  return page.rows.find(match)!;
}

async function waitText(fixture: { nativeElement: HTMLElement }, re: RegExp) {
  await vi.waitFor(() => expect(fixture.nativeElement.textContent).toMatch(re));
}

describe('MatchCounter', () => {
  it('counts events matching conditions', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const counter = TestBed.inject(MatchCounter);
    expect(await counter.count([{ field: 'MachineName', op: 'eq', value: 'WEB01' }])).toBe(7);
    expect(await counter.count([])).toBe(11);
  });
});

describe('DropDialog', () => {
  const initial = [{ field: 'MachineName', op: 'eq' as const, value: 'WEB02' }];

  it('shows the live count and saves a drop step', async () => {
    const { fixture, backend, store } = await mount(DropDialog, { initial });
    await waitText(fixture, /matches\s*4\s*events/);
    expect(text(fixture)).toContain('drop events where MachineName eq "WEB02"');
    buttonByText(fixture, 'Add drop step').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    await applied();
    const toml = (await backend.getProfile()).toml;
    expect(toml).toContain('step = "drop"');
    expect(store.summary()?.total).toBe(7);
    expect(store.notice()).toMatch(/drop step/);
  });

  it('recounts when conditions change and blocks saving without conditions', async () => {
    const { fixture } = await mount(DropDialog, { initial });
    await waitText(fixture, /matches\s*4/);
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label=Value]'), 'WEB01');
    await waitText(fixture, /matches\s*7/);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove condition"]').click();
    await settle(fixture);
    expect(buttonByText(fixture, 'Add drop step').disabled).toBe(true);
    buttonByText(fixture, 'Cancel').click();
    expect(ref.close).toHaveBeenCalled();
  });

  it('shows the error when saving fails and ignores a failing count', async () => {
    const backend = tinyBackend();
    const { fixture } = await mount(DropDialog, { initial }, backend);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue('disk full');
    buttonByText(fixture, 'Add drop step').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('disk full');
    vi.spyOn(backend, 'queryPage').mockRejectedValue('nope');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label=Value]'), 'x');
    await new Promise((r) => setTimeout(r, 350));
    await settle(fixture);
    expect(text(fixture)).toContain('Counting');
  });

  it('reports an invalid profile from validation', async () => {
    const backend = tinyBackend();
    const { fixture } = await mount(DropDialog, { initial }, backend);
    vi.spyOn(backend, 'validateProfile').mockResolvedValue({ ok: false, error: null, line: null });
    buttonByText(fixture, 'Add drop step').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('not valid');
  });
});

describe('MaskDialog', () => {
  it('applyMask handles invalid regexes', () => {
    expect(applyMask('id 12', '\\d+', '#')).toBe('id #');
    expect(applyMask('x', '(', '#')).toBeNull();
  });

  it('previews before/after and saves a mask step', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const rec = await record((r) => r.message.startsWith('Slow request'));
    TestBed.resetTestingModule();
    const { fixture, backend } = await mount(MaskDialog, { record: rec, selected: '1500' });
    expect(el(fixture, '[data-testid=mask-before]').textContent).toBe(rec.message);
    expect(el(fixture, '[data-testid=mask-after]').textContent).toBe('Slow request /api took *** ms');
    await waitText(fixture, /Changes the message of\s*6\s*events/);
    typeInto(el<HTMLInputElement>(fixture, 'input'), '(');
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('not a valid');
    expect(buttonByText(fixture, 'Add mask step').disabled).toBe(true);
    typeInto(el<HTMLInputElement>(fixture, 'input'), '\\d+');
    await settle(fixture);
    buttonByText(fixture, 'Add mask step').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    await applied();
    expect((await backend.getProfile()).toml).toContain('step = "mask"');
  });

  it('shows save errors', async () => {
    const backend = tinyBackend();
    const { fixture } = await mount(MaskDialog, { record: { message: 'a 1' } as RecordView, selected: '1' }, backend);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue(new Error('bad'));
    buttonByText(fixture, 'Add mask step').click();
    await settle(fixture);
    expect(text(fixture)).toContain('bad');
  });
});

describe('MarkDialog', () => {
  const initial = [{ field: '@level', op: 'eq' as const, value: 'Warning' }];

  it('saves a marker rule with a derived kind', async () => {
    const { fixture, backend } = await mount(MarkDialog, { initial, suggestedLabel: 'Slow requests' });
    await waitText(fixture, /marks\s*1\s*events/);
    expect(el<HTMLInputElement>(fixture, 'input.mono').value).toBe('slow-requests');
    typeInto(el<HTMLInputElement>(fixture, 'input[type=color]'), '#ff0000');
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    buttonByText(fixture, 'Add marker').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    await applied();
    const toml = (await backend.getProfile()).toml;
    expect(toml).toContain('kind = "slow-requests"');
    expect(toml).toContain('lifecycle = true');
  });

  it('lets the kind be edited and needs conditions', async () => {
    const { fixture } = await mount(MarkDialog, { initial });
    typeInto(el<HTMLInputElement>(fixture, 'input.mono'), 'custom');
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'input.mono').value).toBe('custom');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove condition"]').click();
    await settle(fixture);
    expect(buttonByText(fixture, 'Add marker').disabled).toBe(true);
  });

  it('shows save errors', async () => {
    const backend = tinyBackend();
    const { fixture } = await mount(MarkDialog, { initial }, backend);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue('nope');
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('nope');
  });
});

describe('ExtractDialog', () => {
  async function slowRecord() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const rec = await record((r) => r.message.startsWith('Slow request'));
    TestBed.resetTestingModule();
    return rec;
  }

  it('proposes a regex, previews matches and saves the rule with a chart', async () => {
    const rec = await slowRecord();
    const start = rec.message.indexOf('1500');
    const { fixture, backend, store } = await mount(ExtractDialog, { record: rec, start, end: start + 4 });
    expect(el<HTMLInputElement>(fixture, 'input.mono').value).toContain('(?P<took>[\\d.]+)');
    await waitText(fixture, /1\s*of\s*\d+\s*candidate events match/);
    expect(el(fixture, '[data-testid=extract-preview]').textContent).toContain('took = 1500');
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    buttonByText(fixture, 'Add extractor').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    await applied();
    const toml = (await backend.getProfile()).toml;
    expect(toml).toContain('[[extract]]');
    expect(toml).toContain('avg(took)');
    expect(store.summary()?.charts.some((c) => c.y === 'avg(took)')).toBe(true);
  });

  it('shows preview errors, type specific fields and disables saving for a bad name', async () => {
    const rec = await slowRecord();
    const start = rec.message.indexOf('1500');
    const { fixture } = await mount(ExtractDialog, { record: rec, start, end: start + 4 });
    typeInto(el<HTMLInputElement>(fixture, 'input.mono'), '(');
    await waitText(fixture, /Invalid|invalid|Unexpected|error/i);
    const select = el<HTMLSelectElement>(fixture, 'select');
    typeInto(select, 'bytes', 'change');
    await settle(fixture);
    expect(text(fixture)).toContain('Store as');
    typeInto(select, 'string', 'change');
    await settle(fixture);
    const name = at(all<HTMLInputElement>(fixture, 'input.mono'), 1);
    typeInto(name, '1bad');
    await settle(fixture);
    expect(buttonByText(fixture, 'Add extractor').disabled).toBe(true);
  });

  it('reports a failing preview request and save errors', async () => {
    const rec = await slowRecord();
    const backend = tinyBackend();
    vi.spyOn(backend, 'previewExtractor').mockRejectedValue('preview down');
    const start = rec.message.indexOf('1500');
    const { fixture } = await mount(ExtractDialog, { record: rec, start, end: start + 4 }, backend);
    await waitText(fixture, /preview down/);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue('no save');
    buttonByText(fixture, 'Add extractor').click();
    await settle(fixture);
    expect(text(fixture)).toContain('no save');
  });

  it('reports when the current profile cannot be loaded', async () => {
    const rec = await slowRecord();
    const backend = tinyBackend();
    vi.spyOn(backend, 'getProfile').mockRejectedValue(new Error('no profile'));
    const { fixture } = await mount(ExtractDialog, { record: rec, start: 0, end: 4 }, backend);
    await settle(fixture);
    expect(text(fixture)).toContain('no profile');
  });
});

describe('selectionWithin', () => {
  it('returns text and offsets within the container', () => {
    const div = document.createElement('div');
    div.textContent = 'hello big world';
    document.body.appendChild(div);
    const range = document.createRange();
    range.setStart(div.firstChild!, 6);
    range.setEnd(div.firstChild!, 9);
    expect(selectionWithin(div, range)).toEqual({ text: 'big', start: 6, end: 9 });
    range.collapse();
    expect(selectionWithin(div, range)).toBeNull();
    div.remove();
  });
});

describe('RecordActions and the context menu', () => {
  async function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(ContextMenuHost);
    await settle(fixture);
    const rec = await record((r) => r.message.startsWith('Slow request'));
    return { fixture, rec, actions: TestBed.inject(RecordActions), menu: TestBed.inject(ContextMenuService), store: TestBed.inject(WorkspaceStore), dialogs: TestBed.inject(DialogService) };
  }

  it('offers row actions and opens the drop dialog prefilled from the record', async () => {
    const { rec, actions, dialogs } = await setup();
    const items = actions.itemsFor(rec);
    // In groups, Copy first (F3): copy · mark · open · rules.
    expect(items.map((i) => i.label)).toEqual(['Copy', 'Highlight lines…', 'Open file', `Open at line ${rec.line}`, 'Show in folder', 'Mark events like this…', 'Drop events like this (pipeline)…', 'Infer pattern from lines…']);
    expect(items.map((i) => i.group)).toEqual(['copy', 'mark', 'open', 'open', 'open', 'rules', 'rules', 'rules']);
    const byLabel = (label: string) => actions.itemsFor(rec).find((i) => i.label === label)!;
    byLabel('Drop events like this (pipeline)…').run();
    expect(dialogs.stack()[0]?.component).toBe(DropDialog);
    expect(dialogs.stack()[0]?.options.inputs?.['initial']).toEqual([{ field: '@message', op: 'eq', value: rec.message }]);
    byLabel('Mark events like this…').run();
    expect(dialogs.stack()[1]?.component).toBe(MarkDialog);
  });

  it('draws a separator between groups and shows shortcuts right-aligned (F3)', async () => {
    const { fixture, menu } = await setup();
    menu.open(10, 10, [
      { group: 'copy', label: 'Copy', run: () => undefined },
      { group: 'mark', label: 'Highlight lines…', run: () => undefined },
      { group: 'mark', label: 'Add note…', shortcut: 'Mod+D', run: () => undefined },
      { group: 'open', label: 'Open file', run: () => undefined },
    ], null);
    await settle(fixture);
    const host: HTMLElement = fixture.nativeElement;
    expect([...host.querySelectorAll('[role=menu] > *')].map((e) => e.getAttribute('role'))).toEqual(['menuitem', 'separator', 'menuitem', 'menuitem', 'separator', 'menuitem']);
    const note = [...host.querySelectorAll<HTMLElement>('[role=menuitem]')].find((b) => b.textContent?.includes('Add note'))!;
    expect(note.querySelector('kbd')?.textContent).toMatch(/^(Ctrl\+D|⌘D)$/);
    expect(note.getAttribute('aria-keyshortcuts')).toMatch(/^(Control|Meta)\+D$/);
    // Keyboard navigation skips the separators.
    expect(document.activeElement?.textContent).toContain('Copy');
    (host.querySelector('[role=menu]') as HTMLElement).dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement?.textContent).toContain('Highlight lines');
    menu.close();
  });

  it('groups contributed entries after the menu own ones, each contribution together (F3)', () => {
    const e = (label: string, group?: string) => ({ label, run: () => undefined, ...(group ? { group } : {}) });
    expect(groupMenu([e('Ask AI', 'contribution-0'), e('Copy', 'copy'), e('Add note…', 'mark'), e('Rule', 'rules'), e('Filter', 'filter'), e('Report', 'contribution-1'), e('Ask AI 2', 'contribution-0'), e('Odd')]).map((x) => x.label)).toEqual([
      'Copy',
      'Filter',
      'Add note…',
      'Rule',
      'Ask AI',
      'Ask AI 2',
      'Report',
      'Odd',
    ]);
  });

  it('offers field and selection actions', async () => {
    const { rec, actions, store, dialogs } = await setup();
    const items = actions.itemsFor(rec, { field: { name: 'MachineName', value: 'WEB01' }, selection: { text: '1500', start: 20, end: 24 } });
    expect(items.map((i) => i.label)).toEqual([
      'Copy',
      'Only this MachineName',
      'Exclude this MachineName',
      'Highlight lines…',
      'Open file',
      `Open at line ${rec.line}`,
      'Show in folder',
      'Mark events like this…',
      'Drop events like this (pipeline)…',
      'Infer pattern from lines…',
      'Mask in message…',
      'Extract value…',
    ]);
    const run = (label: string) => items.find((i) => i.label === label)!.run();
    run('Only this MachineName');
    run('Exclude this MachineName');
    expect(store.query().filters.map((f) => f.op)).toEqual(['eq', 'ne']);
    run('Drop events like this (pipeline)…');
    expect(dialogs.stack()[0]?.options.inputs?.['initial']).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
    run('Mark events like this…');
    expect(dialogs.stack()[1]?.options.inputs?.['suggestedLabel']).toBe('MachineName WEB01');
    run('Highlight lines…');
    run('Infer pattern from lines…');
    run('Mask in message…');
    run('Extract value…');
    expect(dialogs.stack().map((d) => d.component)).toEqual([DropDialog, MarkDialog, MarkDialog, InferDialog, MaskDialog, ExtractDialog]);
    expect(dialogs.stack()[2]?.options.inputs?.['initial']).toEqual([{ field: '@message', op: 'matches', value: '\\d+' }]);
    expect(actions.itemsFor(rec, { selection: { text: '  ', start: 0, end: 2 } })).toHaveLength(8);
  });

  it('infers from every selected record, or from the record alone', async () => {
    const { rec, actions, dialogs } = await setup();
    const other = { ...rec, id: rec.id + 1, message: 'Slow request 2' };
    const infer = (target: Parameters<RecordActions['itemsFor']>[1]) => actions.itemsFor(rec, target).find((i) => i.label === 'Infer pattern from lines…')!.run();
    infer({ records: [rec, other] });
    expect(dialogs.stack()[0]?.options.inputs?.['lines']).toEqual([rec.message, 'Slow request 2']);
    infer({ records: [] });
    expect(dialogs.stack()[1]?.options.inputs?.['lines']).toEqual([rec.message]);
    infer({});
    expect(dialogs.stack()[2]?.options.inputs?.['lines']).toEqual([rec.message]);
  });

  it('infers from the grid multi-selection for the palette command, else from the selected record', async () => {
    const { rec, actions, dialogs, store } = await setup();
    const other = { ...rec, id: rec.id + 1, message: 'Slow request 2' };
    const grid = TestBed.inject(GridSelection);
    store.select(null);
    expect(actions.inferSelected()).toBe(false);
    expect(dialogs.stack()).toHaveLength(0);
    store.select(rec);
    expect(actions.inferSelected()).toBe(true);
    expect(dialogs.stack()[0]?.options.inputs?.['lines']).toEqual([rec.message]);
    grid.publish([rec, other]);
    expect(actions.inferSelected()).toBe(true);
    expect(dialogs.stack()[1]?.options.inputs?.['lines']).toEqual([rec.message, 'Slow request 2']);
  });

  it('offers a logger filter on rows that have one and truncates long labels', async () => {
    const { rec, actions, dialogs } = await setup();
    const withContext = { ...rec, fields: { ...rec.fields, SourceContext: 'Web.Requests' }, template: 'x'.repeat(80) };
    const items = actions.itemsFor(withContext);
    expect(items[1]?.label).toBe('Only this SourceContext');
    actions.mark(withContext);
    expect(String(dialogs.stack()[0]?.options.inputs?.['suggestedLabel'])).toHaveLength(41);
  });

  it('opens at the pointer or below the focused element, with keyboard navigation', async () => {
    const { fixture, rec, actions, menu } = await setup();
    const row = document.createElement('div');
    document.body.appendChild(row);
    actions.openFor(new MouseEvent('contextmenu', { clientX: 30, clientY: 40 }), rec);
    await settle(fixture);
    const popover = el<HTMLElement>(fixture, '[role=menu]');
    expect(popover.style.left).toBe('30px');
    const items = all<HTMLButtonElement>(fixture, '[role=menuitem]');
    expect(document.activeElement).toBe(items[0]);
    const key = (k: string) => popover.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
    key('ArrowDown');
    expect(document.activeElement).toBe(items[1]);
    key('ArrowUp');
    expect(document.activeElement).toBe(items[0]);
    key('ArrowUp');
    expect(document.activeElement).toBe(items[items.length - 1]);
    key('ArrowDown');
    expect(document.activeElement).toBe(items[0]);
    key('End');
    expect(document.activeElement).toBe(items[items.length - 1]);
    key('Home');
    expect(document.activeElement).toBe(items[0]);
    key('x');
    expect(menu.menu()).not.toBeNull();
    row.focus();
    key('Escape');
    await settle(fixture);
    expect(menu.menu()).toBeNull();

    const button = document.createElement('button');
    document.body.appendChild(button);
    button.focus();
    actions.openAtElement(button, rec);
    await settle(fixture);
    el<HTMLElement>(fixture, '[role=menu]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(menu.menu()).toBeNull();
    expect(document.activeElement).toBe(button);

    actions.openFor({ type: 'keydown', currentTarget: button, target: button } as unknown as KeyboardEvent, rec);
    await settle(fixture);
    at(all<HTMLButtonElement>(fixture, '[role=menuitem]'), 0).click();
    await settle(fixture);
    expect(menu.menu()).toBeNull();
    actions.openAtElement(null, rec);
    await settle(fixture);
    el<HTMLElement>(fixture, '.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    await settle(fixture);
    expect(menu.menu()).toBeNull();
    actions.openAtElement(null, rec);
    await settle(fixture);
    el<HTMLElement>(fixture, '.backdrop').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(menu.menu()).toBeNull();
    menu.open(0, 0, []);
    expect(menu.menu()).toBeNull();
    row.remove();
    button.remove();
  });
});

describe('copyTextFor', () => {
  const rec = (message: string) => ({ message, fields: {} }) as unknown as RecordView;

  it('copies the selected text, else the messages of the selected records', () => {
    const one = rec('first line');
    expect(copyTextFor(one, { selection: { text: '1500', start: 0, end: 4 } }, 'elsewhere')).toBe('1500');
    expect(copyTextFor(one, {}, 'grid text')).toBe('grid text');
    expect(copyTextFor(one, { selection: { text: '  ', start: 0, end: 2 } }, '')).toBe('first line');
    expect(copyTextFor(one, { records: [one, rec('second line')] }, ' \n')).toBe('first line\nsecond line');
  });

  it('puts Copy on the record menu and copies through the clipboard', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const items = TestBed.inject(RecordActions).itemsFor(rec('copied message'));
    items.find((i) => i.label === 'Copy')?.run();
    expect(writeText).toHaveBeenCalledWith('copied message');
  });
});
