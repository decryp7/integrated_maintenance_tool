import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { DebriefBackend } from '../core/backend';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveRulesProfile } from '../live/live-rules.profile';
import { newRule } from '../live/live-rules.model';
import { MarkerProfile } from '../markers/marker-profile';
import { ProfileDraft } from '../profile/profile-draft';
import { ProfileEdits } from '../profile/profile-edits';
import { HighlightRules } from '../projects/bookmarks/highlight-rules.store';
import { SavedQueryStore } from '../projects/bookmarks/saved-query.store';
import { DashboardStore } from '../share/dashboard/dashboard.store';
import { ReportStore } from '../share/report/report.store';
import { UndoStack } from './undo-stack';
import { at } from '../../testing/must';

async function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: Platform, useValue: { pickProfileToOpen: async () => null, pickProfileToSave: async () => null, onFilesDropped: async () => () => undefined } }, ProfileDraft] });
  const store = TestBed.inject(WorkspaceStore);
  await settle();
  return { store, undo: TestBed.inject(UndoStack), backend: TestBed.inject(DebriefBackend) };
}

const profileText = async () => (await TestBed.inject(DebriefBackend).getProfile()).toml;

describe('undoable UI-state edits', () => {
  it('highlight rules and saved queries', async () => {
    const { undo, store } = await setup();
    const h = TestBed.inject(HighlightRules);
    h.add({ field: '@level', op: 'eq', value: 'Error', color: '#f00' });
    const id = at(h.items(), 0).id;
    h.update(id, { value: 'Warning' });
    h.remove(id);
    expect(undo.undoLabel()).toBe('highlight rule removal');
    await undo.undo();
    await undo.undo();
    expect(h.items()[0]?.value).toBe('Error');
    await undo.undo();
    expect(h.items()).toEqual([]);

    const q = TestBed.inject(SavedQueryStore);
    store.setTextInput('timeout');
    q.saveCurrent('');
    const qid = at(q.items(), 0).id;
    q.rename(qid, 'Timeouts');
    q.remove(qid);
    await undo.undo();
    expect(q.items()[0]?.name).toBe('Timeouts');
    await undo.undo();
    expect(q.items()[0]?.name).toBe('Query 1');
    await undo.undo();
    expect(q.items()).toEqual([]);
  });

  it('reports: every edit is undoable, merged while typing', async () => {
    const { undo } = await setup();
    const r = TestBed.inject(ReportStore);
    const report = r.create('Incident');
    r.update(report.id, { title: 'Incident 1' });
    r.update(report.id, { title: 'Incident 12' });
    const block = r.add({ type: 'text', markdown: 'x' });
    r.updateBlock(block.id, { markdown: 'xy' } as never);
    const section = r.addSection('More');
    r.renameSection(section.id, 'Later');
    r.add({ type: 'text', markdown: 'second' });
    r.moveBlock(block.id, 1);
    r.removeBlock(block.id);
    r.moveSection(section.id, -1);
    r.removeSection(section.id);
    r.remove(report.id);
    expect(r.reports()).toEqual([]);
    const labels: string[] = [];
    while (undo.canUndo()) {
      labels.push(undo.undoLabel()!);
      await undo.undo();
    }
    expect(labels).toContain('report deletion');
    expect(labels).toContain('report edit');
    expect(labels.filter((l) => l === 'report edit').length).toBe(1);
    expect(r.reports()).toEqual([]);
    // Nothing to do without a report.
    r.updateBlock('nope', {});
    r.removeBlock('nope');
    r.moveBlock('nope', 1);
    r.renameSection('nope', 'x');
    r.removeSection('nope');
    r.moveSection('nope', 1);
    r.update('nope', { title: 'x' });
    expect(undo.canUndo()).toBe(false);
    await undo.redo();
    expect(r.reports().length).toBe(1);
  });
});

describe('undoable profile edits', () => {
  it('dashboards: layout edits undo while editing and are dropped on save; the save is a profile undo', async () => {
    const { undo } = await setup();
    const ds = TestBed.inject(DashboardStore);
    if (!ds.current()) ds.newDashboard('Mine');
    else ds.startEdit();
    ds.setTitle('A');
    ds.setTitle('AB');
    const id = ds.add({ id: 't', title: 'T', type: 'stat', x: 0, y: 0, w: 2, h: 2, query: 'count()' })!;
    ds.resize(id, 1, 0);
    expect(undo.undoLabel()).toBe('dashboard layout change');
    await undo.undo();
    expect(ds.draft()!.tile.find((t) => t.id === id)!.w).toBe(2);
    await undo.undo();
    expect(ds.draft()!.tile.some((t) => t.id === id)).toBe(false);
    expect(undo.undoLabel()).toBe('dashboard title edit');
    await undo.redo();
    ds.remove(id);
    expect(undo.undoLabel()).toBe('dashboard tile removal');
    ds.move('missing', 1, 0);
    const before = await profileText();
    expect(await ds.save()).toBe(true);
    expect(undo.undoLabel()).toBe('dashboard save');
    expect(undo.size()).toBe(1);
    await undo.undo();
    expect(await profileText()).toBe(before);
    // Entries of a cancelled edit go too, and a draft edit undone after the edit ended does nothing.
    ds.startEdit();
    ds.setTitle('Z');
    const stale = undo.undoLabel();
    ds.cancel();
    expect(stale).toBe('dashboard title edit');
    expect(undo.undoLabel()).not.toBe('dashboard title edit');
  });

  it('marker rules, "from data" edits and live rules record the profile before the change', async () => {
    const { undo } = await setup();
    const before = await profileText();
    const res = await TestBed.inject(MarkerProfile).commit((model) => {
      model.name = 'marked';
    });
    expect(res.ok).toBe(true);
    expect(undo.undoLabel()).toBe('marker rule edit');
    await TestBed.inject(ProfileEdits).apply((m) => {
      m.name = 'edited';
    }, 'drop step');
    expect(undo.undoLabel()).toBe('drop step');
    const live = TestBed.inject(LiveRulesProfile);
    const draft = await live.load();
    expect((await live.save({ ...draft, rules: [newRule(0)] })).ok).toBe(true);
    expect(undo.undoLabel()).toBe('live rules edit');
    await undo.undo();
    await undo.undo();
    await undo.undo();
    expect(await profileText()).toBe(before);
  });

  it('a refused marker commit records nothing', async () => {
    const { undo } = await setup();
    const res = await TestBed.inject(MarkerProfile).commit(() => {
      throw new Error('bad');
    });
    expect(res.ok).toBe(false);
    expect(undo.canUndo()).toBe(false);
  });

  it('the profile editor records its save and follows undo when clean, flags stale when dirty', async () => {
    const { undo, store } = await setup();
    const draft = TestBed.inject(ProfileDraft);
    await draft.refresh(); // nothing loaded yet: ignored
    await draft.load();
    const original = draft.text();
    draft.setText(`${original}\n# one\n`);
    await new Promise((r) => setTimeout(r, 350));
    await draft.save();
    expect(undo.undoLabel()).toBe('profile edit');
    await undo.undo();
    await settle();
    await draft.refresh();
    expect(draft.text()).toBe(original);
    expect(draft.dirty()).toBe(false);
    // Same profile again: nothing to take.
    await draft.refresh();
    // Dirty: the edits stay, flagged as based on an older version; Revert goes to the active profile.
    draft.setText(`${original}\n# mine\n`);
    await undo.redo();
    await settle();
    await draft.refresh();
    expect(draft.stale()).toBe(true);
    expect(draft.text()).toContain('# mine');
    expect(store.notice()).toMatch(/based on the previous version/);
    draft.revert();
    await settle();
    expect(draft.stale()).toBe(false);
    expect(draft.text()).toContain('# one');
  });

  it('the profile editor keeps its draft when the active profile cannot be read', async () => {
    const { backend } = await setup();
    const draft = TestBed.inject(ProfileDraft);
    await draft.load();
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce('down');
    await draft.refresh();
    expect(draft.doc()).not.toBeNull();
  });
});
