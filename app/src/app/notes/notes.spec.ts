import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, typeInto } from '../../testing/helpers';
import { at } from '../../testing/must';
import { backendCallStarted, methodKind } from '../core/activity/backend-activity';
import { activity } from '../core/activity/runtime';
import { NoteAnchor, NoteItem, RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { NoteImageStore } from './image/note-image.store';
import { mockImageKey, MockNotesBackend } from './mock/mock-notes-backend';
import { anchors, NOTE_IMAGE_VIEW, NoteActions } from './note-actions';
import { NoteEditor, parseTags } from './note-editor';
import { layoutNoteMarks } from './note-timeline-track';
import { NotesBackend, TauriNotesBackend } from './notes-backend';
import { allTags, anchorSpan, anchorText, byEvent, bySource, firstLine, kindLabel, matches, NO_FILTER, NOTE_KINDS, severityColor, severityLabel, timeNotes } from './notes-model';
import { NotesPanel } from './notes-panel';
import { NotesPersistence } from './notes-persistence';
import { notesCommands, notesRecordMenu, notesViews, provideNotes } from './provide-notes';
import { NotesStore } from './notes.store';
import { NotesInspectorSection } from './notes-inspector-section';
import { noteLabel } from '../grid/events-grid';

const item = (id: string, anchor: NoteAnchor, text = 'seen', patch: Partial<NoteItem> = {}): NoteItem => ({
  id,
  anchor,
  text,
  tags: [],
  severity: 'info',
  author: 'ana',
  createdMs: 1,
  updatedMs: 1,
  resolved: true,
  ...patch,
});

const ctx = () => ({ showView: vi.fn(), jumpTo: vi.fn() }) as unknown as CommandContext;

function provide(backend: NotesBackend = new MockNotesBackend(), context = ctx()) {
  TestBed.configureTestingModule({ providers: [...provideTinyBackend(), { provide: NotesBackend, useValue: backend }, { provide: COMMAND_CONTEXT, useValue: context }] });
  return { backend, context };
}

describe('notes model', () => {
  const notes = [
    item('n1', { kind: 'event', eventId: 7, ts: 50, source: 'a.log', excerpt: 'disk full' }, '# Root cause\nmore', { tags: ['disk'] }),
    item('n2', { kind: 'event', eventId: 7, ts: 50, source: 'a.log', excerpt: '' }),
    item('n3', { kind: 'source', path: '/logs/a.log' }),
    item('n4', { kind: 'range', from: 100, to: 200 }, 'outage', { severity: 'problem', tags: ['net', 'disk'] }),
    item('n5', { kind: 'instant', at: 10 }),
    item('n6', { kind: 'marker', name: 'boot', from: 300, to: null }),
    item('n7', { kind: 'chartPoint', chart: 'CPU', series: 'web01', at: 150, value: 93 }),
    item('n8', { kind: 'project' }),
    item('n9', { kind: 'image', image: { key: 'k', name: 'screen.png', path: null, width: 10, height: 10 }, regions: [{ x: 0, y: 0, w: 1, h: 1 }] }),
  ];

  it('labels every kind and severity', () => {
    expect(NOTE_KINDS.map(kindLabel)).toEqual(['Event', 'Moment', 'Time range', 'Source', 'Marker', 'Chart point', 'Project', 'Image']);
    expect((['info', 'warning', 'problem'] as const).map(severityLabel)).toEqual(['Info', 'Warning', 'Problem']);
    expect(severityColor('problem')).toContain('error');
    expect(severityColor('warning')).toContain('warn');
    // The label is text on a card: it uses the text-safe accent (contrast 4.5:1), not the fill accent.
    expect(severityColor('info')).toBe('var(--accent-text)');
    expect(notes.map((n) => anchorText(n.anchor))).toEqual(['disk full', 'Event 7', 'a.log', 'Time range', 'Moment', 'boot', 'CPU · web01', 'The whole project', 'screen.png']);
    expect(anchorText({ kind: 'chartPoint', chart: 'CPU', series: null, at: 1, value: null })).toBe('CPU');
  });

  it('gives time spans, groups and filters', () => {
    expect(notes.map((n) => anchorSpan(n.anchor))).toEqual([[50, 50], [50, 50], null, [100, 200], [10, 10], [300, 300], [150, 150], null, null]);
    expect(anchorSpan({ kind: 'event', eventId: 1, ts: null, source: '', excerpt: '' })).toBeNull();
    expect(anchorSpan({ kind: 'marker', name: 'run', from: 1, to: 5 })).toEqual([1, 5]);
    expect(byEvent(notes).get(7)?.map((n) => n.id)).toEqual(['n1', 'n2']);
    expect(bySource(notes).get('/logs/a.log')).toBe(1);
    expect(allTags(notes)).toEqual(['disk', 'net']);
    expect(notes.filter((n) => matches(n, { ...NO_FILTER, tag: 'disk' })).map((n) => n.id)).toEqual(['n1', 'n4']);
    expect(notes.filter((n) => matches(n, { ...NO_FILTER, kind: 'range' })).map((n) => n.id)).toEqual(['n4']);
    expect(notes.filter((n) => matches(n, { ...NO_FILTER, text: 'ROOT disk' })).map((n) => n.id)).toEqual(['n1']);
    expect(notes.filter((n) => matches(n, NO_FILTER))).toHaveLength(notes.length);
  });

  it('puts time notes (not event notes) of the visible range on the timeline', () => {
    expect(timeNotes(notes, 0, 250).map((t) => t.id)).toEqual(['n4', 'n5', 'n7']);
    expect(at(timeNotes(notes, 0, 250), 0)).toMatchObject({ from: 100, to: 200, severity: 'problem', label: 'outage' });
    const marks = layoutNoteMarks(timeNotes(notes, 0, 400), { from: 0, to: 400 }, 400);
    expect(marks.map((m) => [m.note.id, m.left, m.width])).toEqual([['n5', 2.5, 0], ['n4', 25, 25], ['n7', 37.5, 0], ['n6', 75, 0]]);
    expect(marks.map((m) => m.row)).toEqual([0, 0, 1, 0]);
    // Labels that would overlap stack.
    const same = [item('x1', { kind: 'instant', at: 10 }), item('x2', { kind: 'instant', at: 11 })];
    expect(layoutNoteMarks(timeNotes(same, 0, 100), { from: 0, to: 100 }).map((m) => m.row)).toEqual([0, 1]);
    // A range that starts before the view is clipped to it.
    expect(at(layoutNoteMarks(timeNotes(notes, 150, 400), { from: 150, to: 400 }), 0)).toMatchObject({ left: 0 });
  });

  it('takes the first line without Markdown marks', () => {
    expect(firstLine('\n## Heading\nbody')).toBe('Heading');
    expect(firstLine('- item')).toBe('item');
    expect(firstLine('**Root cause**: `df` said __full__')).toBe('Root cause: df said full');
    expect(firstLine('x'.repeat(100), 10)).toBe(`${'x'.repeat(9)}…`);
    expect(parseTags(' #disk, net  io,')).toEqual(['disk', 'net', 'io']);
    // The grid's glyph: the first note's first line and how many more.
    expect(noteLabel(undefined)).toBeNull();
    expect(noteLabel([{ text: '# Root cause' }])).toBe('Root cause');
    expect(noteLabel([{ text: '---' }, { text: 'b' }])).toBe('--- (+1 more)');
    expect(noteLabel([{ text: '- ' }])).toBe('Note');
  });
});

describe('TauriNotesBackend', () => {
  it('calls the notes commands and names its calls without text', async () => {
    const call = vi.fn().mockResolvedValue({});
    const b = new TauriNotesBackend(call as never);
    const draft = { anchor: anchors.project(), text: 'SECRET TEXT' };
    await b.listNotes();
    await b.addNote(draft);
    await b.updateNote('n1', { text: 'SECRET' });
    await b.removeNote('n1');
    await b.getNotesRecord();
    await b.restoreNotes(undefined);
    await b.openNoteImage('/a.png');
    await b.getNoteImage('k');
    expect(call.mock.calls).toEqual([
      ['notes_list'],
      ['notes_add', { draft }],
      ['notes_update', { id: 'n1', change: { text: 'SECRET' } }],
      ['notes_remove', { id: 'n1' }],
      ['notes_record'],
      ['notes_restore', { record: null }],
      ['notes_image_open', { path: '/a.png' }],
      ['notes_image_get', { key: 'k' }],
    ]);
    expect(['listNotes', 'getNotesRecord', 'getNoteImage'].map(methodKind)).toEqual(['query', 'query', 'query']);
    activity.reset();
    vi.spyOn(activity.triggers, 'infer').mockReturnValue('menu');
    const begin = vi.spyOn(activity, 'begin');
    backendCallStarted('updateNote', ['n1', { text: 'SECRET', tags: ['x'] }]);
    activity.reset();
    backendCallStarted('addNote', [draft]);
    activity.reset();
    backendCallStarted('removeNote', ['n1']);
    const params = JSON.stringify(begin.mock.calls);
    expect(params).toContain('notes.add');
    expect(params).toContain('"Kind":"project"');
    expect(params).toContain('text,tags');
    expect(params).not.toContain('SECRET');
    activity.reset();
  });
});

describe('MockNotesBackend', () => {
  it('mimics the engine: checks, image regions, records', async () => {
    const m = new MockNotesBackend();
    await expect(m.addNote({ anchor: anchors.project(), text: ' ' })).rejects.toMatchObject({ code: 'invalid' });
    await expect(m.addNote({ anchor: { kind: 'range', from: 2, to: 1 }, text: 'x' })).rejects.toMatchObject({ code: 'invalid' });
    await expect(m.addNote({ anchor: anchors.project(), text: 'x'.repeat(16 * 1024 + 1) })).rejects.toMatchObject({ code: 'limit' });
    const n = await m.addNote({ anchor: anchors.project(), text: 'a\r\nb ', tags: ['#Disk', 'disk'] });
    expect(n).toMatchObject({ id: 'n1', text: 'a\nb', tags: ['disk'], severity: 'info' });
    const img = await m.openNoteImage('C:\\shots\\screen.png');
    expect(img).toMatchObject({ key: mockImageKey('C:\\shots\\screen.png'), name: 'screen.png', width: 800 });
    expect(img.key).toMatch(/^[0-9a-f]{64}$/);
    const imageAnchor = (x: number, key = img.key): NoteAnchor => ({ kind: 'image', image: { key, name: img.name, path: img.path, width: img.width, height: img.height }, regions: [{ x, y: 0, w: 10, h: 10 }] });
    await expect(m.addNote({ anchor: imageAnchor(795), text: 'x' })).rejects.toMatchObject({ code: 'invalid' });
    const onImage = await m.addNote({ anchor: imageAnchor(10), text: 'banner' });
    await expect(m.updateNote(onImage.id, { regions: [] })).rejects.toMatchObject({ code: 'invalid' });
    await expect(m.updateNote(n.id, { regions: [{ x: 0, y: 0, w: 1, h: 1 }] })).rejects.toMatchObject({ code: 'invalid' });
    expect((await m.updateNote(onImage.id, { regions: [{ x: 1, y: 1, w: 2, h: 2 }], severity: 'warning' })).severity).toBe('warning');
    await expect(m.addNote({ anchor: imageAnchor(1, 'gone'), text: 'x' })).rejects.toMatchObject({ code: 'notFound' });
    expect(await m.getNoteImage(img.key)).toMatchObject({ name: 'screen.png' });
    await expect(m.getNoteImage('nope')).rejects.toMatchObject({ code: 'notFound' });
    const rec = await m.getNotesRecord();
    expect(rec.notes).toHaveLength(2);
    expect(await m.restoreNotes({ version: 1 })).toMatchObject({ count: 0, warning: expect.stringContaining('damaged') });
    expect(await m.restoreNotes(rec)).toEqual({ count: 2, warning: null });
    expect((await m.addNote({ anchor: anchors.project(), text: 'next' })).id).toBe('n3');
    expect(await m.removeNote('n1')).toBe(true);
    await expect(m.removeNote('n1')).rejects.toMatchObject({ code: 'notFound' });
    expect(await m.restoreNotes(null)).toEqual({ count: 0, warning: null });
  });
});

describe('NotesStore and NotesPersistence', () => {
  it('lists, changes and saves notes with the project, and restores them', async () => {
    const { backend } = provide();
    const store = TestBed.inject(NotesStore);
    const persistence = TestBed.inject(NotesPersistence);
    const registry = TestBed.inject(UiStateRegistry);
    registry.register(persistence.contribution);
    await settle();
    expect(store.items()).toEqual([]);
    const n = await store.add({ anchor: anchors.event({ id: 4, ts: 9, sourceName: 'a.log', message: 'boom\nstack' } as RecordView), text: 'first failure', tags: ['crash'] });
    expect(n?.anchor).toMatchObject({ kind: 'event', eventId: 4, excerpt: 'boom' });
    expect(store.byEvent().get(4)).toHaveLength(1);
    expect(store.tags()).toEqual(['crash']);
    expect(registry.serialize()['notes']).toMatchObject({ version: 1, notes: [{ text: 'first failure' }] });
    await store.update(n!.id, { text: 'second look' });
    expect(at(store.items(), 0).text).toBe('second look');
    store.setFilter({ text: 'nothing' });
    expect(store.filtered()).toEqual([]);
    store.clearFilter();
    expect(store.filtered()).toHaveLength(1);
    // Refusals are shown, not thrown.
    expect(await store.add({ anchor: anchors.project(), text: '' })).toBeNull();
    expect(store.actionError()).toContain('write something');
    store.dismissError();
    expect(store.actionError()).toBeNull();
    // A saved project's notes go back to the engine.
    const saved = registry.serialize();
    await store.remove(n!.id);
    expect(store.items()).toEqual([]);
    expect(registry.serialize()['notes']).toBeNull();
    registry.restore(saved);
    await settle();
    expect(store.items().map((x) => x.text)).toEqual(['second look']);
    // A project without notes clears them; a damaged record says why.
    registry.restore({});
    await settle();
    expect(store.items()).toEqual([]);
    await persistence.restore({ version: 1, notes: 'x' });
    expect(persistence.warning()).toContain('damaged');
    // The engine refusing a record keeps it for saving as it was.
    vi.spyOn(backend, 'restoreNotes').mockRejectedValueOnce(new Error('down'));
    await persistence.restore({ version: 1, notes: [] });
    expect(persistence.record()).toEqual({ version: 1, notes: [] });
    vi.spyOn(backend, 'getNotesRecord').mockRejectedValueOnce(new Error('down'));
    await persistence.sync();
    vi.spyOn(backend, 'listNotes').mockRejectedValueOnce(new Error('list failed'));
    await store.load();
    expect(store.loadError()).toContain('list failed');
  });
});

describe('NoteActions', () => {
  it('writes, edits, deletes after confirming, and jumps to anchors', async () => {
    const { backend, context } = provide();
    const actions = TestBed.inject(NoteActions);
    const store = TestBed.inject(NotesStore);
    const dialogs = TestBed.inject(DialogService);
    const ws = TestBed.inject(WorkspaceStore);
    await settle();
    const written = actions.write(anchors.instant(5.4));
    const dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('Add a note');
    expect(dlg?.options.inputs).toEqual({ anchor: { kind: 'instant', at: 5 } });
    dlg?.ref.close('n1');
    expect(await written).toBe('n1');
    expect(await actions.writeOnSelected()).toBe(false);
    expect(await actions.writeOnRange()).toBe(false);
    const n = await store.add({ anchor: anchors.range(9, 3), text: 'window' });
    expect(n?.anchor).toEqual({ kind: 'range', from: 3, to: 9 });
    const it = at(store.items(), 0);
    const edited = actions.edit(it);
    expect(dialogs.stack().at(-1)?.options.title).toBe('Edit note');
    dialogs.stack().at(-1)?.ref.close();
    expect(await edited).toBeNull();
    // Delete asks first.
    let removed = actions.remove(it);
    dialogs.stack().at(-1)?.ref.close();
    expect(await removed).toBe(false);
    removed = actions.remove(it);
    expect(String(dialogs.stack().at(-1)?.options.inputs?.['message'])).toContain('window');
    dialogs.stack().at(-1)?.ref.close(true);
    expect(await removed).toBe(true);
    // Jumps.
    const setRange = vi.spyOn(ws, 'setRange');
    const jumpTo = vi.spyOn(ws, 'jumpTo');
    await actions.jump(item('a', { kind: 'event', eventId: 3, ts: null, source: '', excerpt: '' }));
    expect(jumpTo).toHaveBeenCalledWith(3);
    await actions.jump(item('b', { kind: 'range', from: 1, to: 2 }));
    expect(setRange).toHaveBeenLastCalledWith(1, 2);
    await actions.jump(item('c', { kind: 'instant', at: 100_000 }));
    expect(setRange).toHaveBeenLastCalledWith(70_000, 130_000);
    await actions.jump(item('d', { kind: 'marker', name: 'm', from: 100_000, to: 200_000 }));
    expect(setRange).toHaveBeenLastCalledWith(70_000, 230_000);
    await actions.jump(item('e', { kind: 'source', path: '/nowhere.log' }));
    await actions.jump(item('f', { kind: 'project' }));
    expect(context.showView).toHaveBeenLastCalledWith('notes');
    const img = await backend.openNoteImage('/a.png');
    await actions.jump(item('g', { kind: 'image', image: { ...img }, regions: [{ x: 0, y: 0, w: 1, h: 1 }] }));
    expect(context.showView).toHaveBeenLastCalledWith(NOTE_IMAGE_VIEW);
    expect(TestBed.inject(NoteImageStore).focused()).toBe('g');
  });
});

describe('NoteEditor', () => {
  function open(inputs: Record<string, unknown>) {
    provide();
    const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
    TestBed.configureTestingModule({ providers: [{ provide: DIALOG_REF, useValue: ref }] });
    const f = TestBed.createComponent(NoteEditor);
    for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
    return { f, ref };
  }

  it('writes a note with tags and severity, previews Markdown as text and shows refusals', async () => {
    const { f, ref } = open({ anchor: anchors.project() });
    await settle(f);
    expect(el(f, '[data-testid=note-anchor]').textContent).toContain('The whole project');
    const save = el<HTMLButtonElement>(f, '[data-testid=note-save]');
    expect(save.disabled).toBe(true);
    typeInto(el<HTMLTextAreaElement>(f, '[data-testid=note-text]'), '**Bold** <img src=x onerror=alert(1)>');
    typeInto(el<HTMLInputElement>(f, '[data-testid=note-tags]'), 'disk, #net');
    typeInto(el<HTMLSelectElement>(f, '[data-testid=note-severity]'), 'warning', 'change');
    await settle(f);
    el<HTMLButtonElement>(f, '[data-testid=note-preview-tab]').click();
    await settle(f);
    expect(f.nativeElement.querySelector('img')).toBeNull();
    expect(el(f, '[data-testid=note-preview]').textContent).toContain('<img src=x');
    el<HTMLButtonElement>(f, '[data-testid=note-save]').click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledWith('n1');
    expect(at(TestBed.inject(NotesStore).items(), 0)).toMatchObject({ tags: ['disk', 'net'], severity: 'warning' });
  });

  it('edits a note and keeps the dialog open with the reason when the engine refuses', async () => {
    const store = () => TestBed.inject(NotesStore);
    const { f, ref } = open({});
    await settle(f);
    const n = await store().add({ anchor: anchors.instant(1), text: 'old', tags: ['a'], severity: 'problem' });
    f.componentRef.setInput('note', at(store().items(), 0));
    f.componentInstance.ngOnInit();
    await settle(f);
    expect(el<HTMLTextAreaElement>(f, '[data-testid=note-text]').value).toBe('old');
    expect(el<HTMLInputElement>(f, '[data-testid=note-tags]').value).toBe('a');
    typeInto(el<HTMLTextAreaElement>(f, '[data-testid=note-text]'), 'x'.repeat(16 * 1024 + 1));
    await settle(f);
    expect(el<HTMLButtonElement>(f, '[data-testid=note-save]').disabled).toBe(true);
    typeInto(el<HTMLInputElement>(f, '[data-testid=note-tags]'), 'two words');
    typeInto(el<HTMLTextAreaElement>(f, '[data-testid=note-text]'), 'new');
    vi.spyOn(TestBed.inject(NotesBackend), 'updateNote').mockRejectedValueOnce(new Error('a tag is 1 to 32 letters'));
    await f.componentInstance.save();
    await settle(f);
    expect(el(f, '[data-testid=note-editor-error]').textContent).toContain('a tag is');
    await f.componentInstance.save();
    expect(ref.close).toHaveBeenCalledWith(n!.id);
    expect(at(store().items(), 0).text).toBe('new');
  });
});

describe('NotesPanel', () => {
  it('lists, filters, jumps, edits and deletes notes', async () => {
    const { context } = provide();
    const store = TestBed.inject(NotesStore);
    await store.add({ anchor: anchors.range(1_000, 2_000), text: '## Outage\nDB down', tags: ['db'], severity: 'problem' });
    await store.add({ anchor: anchors.project(), text: 'Customer: ACME' });
    const f = TestBed.createComponent(NotesPanel);
    await settle(f);
    expect(all(f, '[data-testid=note-item]')).toHaveLength(2);
    expect(text(f)).toContain('Outage');
    expect(text(f)).toContain('#db');
    typeInto(el<HTMLInputElement>(f, '[data-testid=notes-filter]'), 'acme');
    await settle(f);
    expect(all(f, '[data-testid=note-item]')).toHaveLength(1);
    typeInto(el<HTMLInputElement>(f, '[data-testid=notes-filter]'), 'zzz');
    await settle(f);
    expect(text(f)).toContain('No note matches the filter.');
    buttonByText(f, 'Clear the filter').click();
    await settle(f);
    typeInto(el<HTMLSelectElement>(f, '[data-testid=notes-kind]'), 'range', 'change');
    await settle(f);
    expect(all(f, '[data-testid=note-item]')).toHaveLength(1);
    typeInto(el<HTMLSelectElement>(f, '[data-testid=notes-kind]'), '', 'change');
    typeInto(el<HTMLSelectElement>(f, '[data-testid=notes-tag]'), 'db', 'change');
    await settle(f);
    expect(all(f, '[data-testid=note-item]')).toHaveLength(1);
    typeInto(el<HTMLSelectElement>(f, '[data-testid=notes-tag]'), '', 'change');
    await settle(f);
    const ws = TestBed.inject(WorkspaceStore);
    const setRange = vi.spyOn(ws, 'setRange');
    at(all<HTMLButtonElement>(f, '[data-testid=note-jump]'), 0).click();
    expect(setRange).toHaveBeenCalledWith(1_000, 2_000);
    expect(context.showView).toHaveBeenCalledWith('events');
    const dialogs = TestBed.inject(DialogService);
    at(all<HTMLButtonElement>(f, '[data-testid=note-edit]'), 0).click();
    expect(dialogs.stack().at(-1)?.options.title).toBe('Edit note');
    dialogs.stack().at(-1)?.ref.close();
    at(all<HTMLButtonElement>(f, '[data-testid=note-delete]'), 1).click();
    dialogs.stack().at(-1)?.ref.close(true);
    await settle(f);
    expect(all(f, '[data-testid=note-item]')).toHaveLength(1);
    el<HTMLButtonElement>(f, '[data-testid=notes-add-project]').click();
    expect(dialogs.stack().at(-1)?.options.inputs).toEqual({ anchor: { kind: 'project' } });
  });

  it('says when there are no notes, and when a list or restore failed', async () => {
    const { backend } = provide();
    const failing = vi.spyOn(backend, 'listNotes').mockRejectedValue(new Error('engine busy'));
    const f = TestBed.createComponent(NotesPanel);
    await settle(f);
    expect(text(f)).toContain('engine busy');
    failing.mockRestore();
    buttonByText(f, 'Retry').click();
    await settle(f);
    expect(el(f, '[data-testid=notes-empty]')).toBeTruthy();
    TestBed.inject(NotesPersistence).warning.set('2 of the project’s notes could not be loaded.');
    await settle(f);
    expect(text(f)).toContain('could not be loaded');
  });
});

describe('NotesPanel drop (drag and drop between views)', () => {
  /** A drag event with a DataTransfer-like object (jsdom has no DataTransfer). */
  function dragEvent(type: string, data: Record<string, string>): DragEvent {
    const e = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
    const store = new Map(Object.entries(data));
    Object.defineProperty(e, 'dataTransfer', { value: { types: [...store.keys()], getData: (t: string) => store.get(t) ?? '', dropEffect: 'none' } });
    return e;
  }
  const payload = JSON.stringify({ id: 9, ts: 5_000, source: 'app.log', excerpt: 'boom' });

  it('accepts a dragged event, marks the panel, and writes a note on it when dropped', async () => {
    provide();
    const f = TestBed.createComponent(NotesPanel);
    await settle(f);
    const host = f.nativeElement as HTMLElement;
    const over = dragEvent('dragover', { 'application/x-debrief-event': payload });
    host.dispatchEvent(over);
    expect(over.defaultPrevented).toBe(true);
    await settle(f);
    expect(host.classList.contains('drop-target')).toBe(true);
    host.dispatchEvent(new Event('dragleave'));
    await settle(f);
    expect(host.classList.contains('drop-target')).toBe(false);
    const drop = dragEvent('drop', { 'application/x-debrief-event': payload });
    host.dispatchEvent(drop);
    expect(drop.defaultPrevented).toBe(true);
    const dialog = TestBed.inject(DialogService).stack().at(-1);
    expect(dialog?.options.title).toBe('Add a note');
    expect(dialog?.options.inputs).toEqual({ anchor: { kind: 'event', eventId: 9, ts: 5_000, source: 'app.log', excerpt: 'boom' } });
  });

  it('ignores other drags and malformed drops', async () => {
    provide();
    const f = TestBed.createComponent(NotesPanel);
    await settle(f);
    const host = f.nativeElement as HTMLElement;
    const over = dragEvent('dragover', { 'application/x-other': 'hi' });
    host.dispatchEvent(over);
    expect(over.defaultPrevented).toBe(false);
    const drop = dragEvent('drop', { 'application/x-debrief-event': '{"id":"x"}' });
    host.dispatchEvent(drop);
    expect(drop.defaultPrevented).toBe(false);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(0);
  });
});

describe('notes contributions', () => {
  it('registers views, commands, the record menu and the inspector section', async () => {
    const { context } = provide();
    TestBed.configureTestingModule({ providers: [provideNotes()] });
    expect(notesViews().map((v) => v.id)).toEqual(['notes', 'note-image']);
    const actions = TestBed.inject(NoteActions);
    const ws = TestBed.inject(WorkspaceStore);
    const cmds = notesCommands(actions, ws);
    expect(cmds.map((c) => c.id)).toEqual(['notes.show', 'notes.addSelected', 'notes.addRange', 'notes.addProject', 'notes.image']);
    expect(at(cmds, 1).keybinding).toBe('Mod+D');
    expect(at(cmds, 1).when?.(context)).toBe(false);
    expect(at(cmds, 2).when?.(context)).toBe(false);
    at(cmds, 0).run(context);
    at(cmds, 4).run(context);
    expect(context.showView).toHaveBeenCalledWith('note-image');
    const write = vi.spyOn(actions, 'write').mockResolvedValue(null);
    at(cmds, 3).run(context);
    expect(write).toHaveBeenCalledWith({ kind: 'project' });
    const menu = notesRecordMenu(actions)({ id: 9, ts: 1, sourceName: 's', message: 'm' } as RecordView, {});
    expect(at(menu, 0).label).toBe('Add note…');
    at(menu, 0).run();
    expect(write).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'event', eventId: 9 }));
    expect(Object.keys(TestBed.inject(UiStateRegistry).serialize())).toContain('notes');
  });

  it('shows the notes of the inspected event and adds one', async () => {
    provide();
    const store = TestBed.inject(NotesStore);
    await store.add({ anchor: { kind: 'event', eventId: 3, ts: 1, source: 's', excerpt: 'e' }, text: 'checked' });
    const f = TestBed.createComponent(NotesInspectorSection);
    f.componentRef.setInput('record', { id: 3, ts: 1, sourceName: 's', message: 'e' } as RecordView);
    await settle(f);
    expect(all(f, '[data-testid=inspector-note]')).toHaveLength(1);
    const write = vi.spyOn(TestBed.inject(NoteActions), 'write').mockResolvedValue(null);
    el<HTMLButtonElement>(f, '[data-testid=inspector-add-note]').click();
    expect(write).toHaveBeenCalledWith(expect.objectContaining({ kind: 'event', eventId: 3 }));
  });
});
