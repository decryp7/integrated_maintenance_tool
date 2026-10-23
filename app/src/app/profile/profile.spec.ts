import { Component, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { parse } from 'smol-toml';
import { all, buttonByText, el, provideTinyBackend, settle, testLogger, text, tinyBackend, typeInto } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { MOCK_PROFILE_TOML } from '../core/mock/mock-profile';
import { Platform } from '../core/platform';
import { Condition, ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { ConditionsEditor } from './conditions-editor';
import { ProfileDraft } from './profile-draft';
import { ProfileEditor } from './profile-editor';
import { ProfileForm } from './profile-form';
import { StringList } from './string-list';
import { TomlEditor } from './toml-editor';
import { CommandRegistry } from '../extensions/command-registry';
import { at } from '../../testing/must';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

const fakePlatform = {
  isTauri: () => false,
  pickPaths: async () => [],
  onFilesDropped: async () => () => undefined,
  openPath: '/p/in.toml' as string | null,
  savePath: '/p/out.toml' as string | null,
  async pickProfileToOpen() {
    return this.openPath;
  },
  async pickProfileToSave() {
    return this.savePath;
  },
};

function configure(backend: ImtBackend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: Platform, useValue: fakePlatform }, ProfileDraft] });
  return { backend, store: TestBed.inject(WorkspaceStore), draft: TestBed.inject(ProfileDraft) };
}

const wait = (ms = 360) => new Promise((r) => setTimeout(r, ms));

/** Opens the "..." menu of the profile editor and triggers the item with this label. */
async function menuItem(fixture: ComponentFixture<unknown>, label: string): Promise<void> {
  (fixture.nativeElement.querySelector('[data-testid=profile-more]') as HTMLButtonElement).click();
  fixture.detectChanges();
  await fixture.whenStable();
  const item = [...document.querySelectorAll<HTMLButtonElement>('.cdk-overlay-container [cdkMenuItem]')].find((b) => b.textContent?.trim() === label);
  if (!item) throw new Error(`No menu item "${label}"`);
  item.click();
}

describe('ProfileDraft', () => {
  it('loads, tracks dirty state and validates', async () => {
    const { draft } = configure();
    await draft.load();
    expect(draft.text()).toBe(MOCK_PROFILE_TOML);
    expect(draft.model().name).toContain('demo');
    expect(draft.dirty()).toBe(false);
    await wait();
    expect(draft.validation()?.ok).toBe(true);
    expect(draft.problem()).toBeNull();
    draft.setText('name = "x"\n[broken');
    expect(draft.dirty()).toBe(true);
    expect(draft.parseError()?.line).toBe(2);
    expect(draft.problem()?.line).toBe(2);
    draft.setText('[[chart]]\nid = "c"\ny = "median(x)"');
    await wait();
    expect(draft.problem()?.message).toMatch(/avg\(field\)/);
    draft.revert();
    expect(draft.dirty()).toBe(false);
  });

  it('syncs form edits back to TOML without undefined values', async () => {
    const { draft } = configure();
    await draft.load();
    draft.update((m) => {
      m.name = 'Edited';
      (m as Record<string, unknown>)['custom_key'] = { keep: true, drop: undefined };
      m.marker = [{ kind: 'k', label: undefined, when: [] }];
    });
    const reparsed = parse(draft.text()) as ProfileModel & { custom_key: unknown };
    expect(reparsed.name).toBe('Edited');
    expect(reparsed.custom_key).toEqual({ keep: true });
    expect(reparsed.marker).toEqual([{ kind: 'k', when: [] }]);
    expect(reparsed.instance?.candidates?.[0]).toBe('Application');
    expect(draft.dirty()).toBe(true);
  });

  it('saves and applies, refusing invalid drafts', async () => {
    const { draft, store, backend } = configure();
    await settle();
    await draft.load();
    draft.update((m) => (m.name = 'Saved profile'));
    await draft.save();
    await applied();
    expect(store.summary()?.profileName).toBe('Saved profile');
    expect(draft.dirty()).toBe(false);
    expect(draft.doc()?.name).toBe('Saved profile');
    expect(testLogger().entries.find((e) => e.category === 'ui.profile')).toMatchObject({ message: 'Profile saved', props: { Bytes: draft.text().length } });

    draft.setText('[[broken');
    const spy = vi.spyOn(backend, 'saveProfile');
    await draft.save();
    await applied();
    expect(spy).not.toHaveBeenCalled();

    draft.setText('name = "ok"');
    spy.mockRejectedValueOnce('disk full');
    await draft.save();
    await applied();
    expect(store.error()).toBe('Profile not saved: disk full');
    expect(draft.saving()).toBe(false);
  });

  it('imports and exports via the platform dialogs', async () => {
    const { draft, backend, store } = configure();
    await draft.load();
    await draft.importFile();
    expect(draft.importedFrom()).toBe('/p/in.toml');
    const exp = vi.spyOn(backend, 'exportProfile').mockResolvedValue();
    draft.update((m) => (m.name = '  My / Profile!! '));
    await draft.exportFile();
    expect(exp).toHaveBeenCalledWith('/p/out.toml', draft.text(), false);
    fakePlatform.openPath = null;
    fakePlatform.savePath = null;
    await draft.importFile();
    await draft.exportFile();
    expect(exp).toHaveBeenCalledTimes(1);
    fakePlatform.openPath = '/p/in.toml';
    fakePlatform.savePath = '/p/out.toml';
    vi.spyOn(backend, 'importProfile').mockRejectedValueOnce('nope');
    await draft.importFile();
    expect(store.error()).toBe('Import failed: nope');
    exp.mockRejectedValueOnce('ro');
    await draft.exportFile();
    expect(store.error()).toBe('Export failed: ro');
    draft.update((m) => delete m.name);
    exp.mockResolvedValue();
    await draft.exportFile();
  });

  it('reports load and validation failures', async () => {
    const backend = tinyBackend();
    const { draft, store } = configure(backend);
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce('no profile');
    await draft.load();
    expect(store.error()).toBe('no profile');
    vi.spyOn(backend, 'validateProfile').mockRejectedValueOnce('validator down');
    draft.setText('name = "x"');
    await wait();
    expect(draft.problem()?.message).toBe('validator down');
    draft.revert();
  });
});

@Component({
  imports: [StringList, ConditionsEditor],
  template: `
    <imt-string-list label="glob" [values]="values()" (changed)="values.set($event)" />
    <imt-conditions-editor [conditions]="conds()" (changed)="conds.set($event)" />
  `,
})
class ListHost {
  readonly values = signal(['a', 'b']);
  readonly conds = signal<Condition[]>([{ field: 'x', op: 'eq', value: '1' }]);
}

describe('StringList and ConditionsEditor', () => {
  it('edit, reorder, add and remove items', async () => {
    const fixture = TestBed.createComponent(ListHost);
    await settle(fixture);
    const host = fixture.componentInstance;
    typeInto(at(all<HTMLInputElement>(fixture, 'imt-string-list .input'), 1), 'B');
    expect(host.values()).toEqual(['a', 'B']);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Move glob 2 up"]').click();
    expect(host.values()).toEqual(['B', 'a']);
    await settle(fixture);
    buttonByText(fixture, '+ Add glob').click();
    expect(host.values()).toEqual(['B', 'a', '']);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove glob 1"]').click();
    expect(host.values()).toEqual(['a', '']);

    const cond = el(fixture, 'imt-conditions-editor');
    typeInto(cond.querySelector<HTMLInputElement>('.field')!, 'Level');
    await settle(fixture);
    typeInto(cond.querySelector<HTMLInputElement>('.value')!, '2');
    expect(host.conds()).toEqual([{ field: 'Level', op: 'eq', value: '2' }]);
    await settle(fixture);
    typeInto(cond.querySelector('select')!, 'exists', 'change');
    expect(host.conds()).toEqual([{ field: 'Level', op: 'exists' }]);
    await settle(fixture);
    expect(cond.querySelector('.value input')).toBeNull();
    buttonByText(fixture, '+ Condition').click();
    expect(host.conds()).toHaveLength(2);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove condition"]').click();
    expect(host.conds()).toHaveLength(1);
    host.conds.set([]);
    await settle(fixture);
    expect(text(fixture)).toContain('Applies to every event');
  });
});

describe('ProfileEditor', () => {
  async function render() {
    const ctx = configure();
    const fixture = TestBed.createComponent(ProfileEditor);
    await settle(fixture);
    await wait();
    await settle(fixture);
    const draft = fixture.debugElement.injector.get(ProfileDraft);
    return { ...ctx, fixture, draft };
  }

  it('shows status and switches between form and TOML', async () => {
    const { fixture, draft } = await render();
    expect(text(fixture)).toContain('Valid');
    expect(text(fixture)).toContain('built-in default');
    expect(buttonByText(fixture, 'Save & apply').disabled).toBe(true);
    buttonByText(fixture, 'TOML').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-toml-editor')).not.toBeNull();
    draft.setText('[broken');
    await settle(fixture);
    expect(text(fixture)).toContain('Line 1');
    buttonByText(fixture, 'Form').click();
    await settle(fixture);
    expect(text(fixture)).toContain("syntax error, so the form can't show it");
    buttonByText(fixture, 'Fix in TOML view').click();
    await settle(fixture);
    await menuItem(fixture, 'Revert');
    await settle(fixture);
    expect(draft.dirty()).toBe(false);
  });

  it('saves with the button and Ctrl/Cmd+S, and imports/exports', async () => {
    const { fixture, draft, store, backend } = await render();
    draft.update((m) => (m.name = 'Via button'));
    await settle(fixture);
    expect(text(fixture)).toContain('Unsaved');
    buttonByText(fixture, 'Save & apply').click();
    await settle(fixture);
    await applied(fixture);
    expect(store.summary()?.profileName).toBe('Via button');
    draft.update((m) => (m.name = 'Via keyboard'));
    fixture.nativeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 's', metaKey: true, bubbles: true }));
    await settle(fixture);
    await applied(fixture);
    expect(store.summary()?.profileName).toBe('Via keyboard');
    fixture.nativeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true }));
    fixture.nativeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'x', ctrlKey: true, bubbles: true }));
    await menuItem(fixture, 'Import…');
    await settle(fixture);
    expect(text(fixture)).toContain('/p/in.toml');
    vi.spyOn(backend, 'exportProfile').mockResolvedValue();
    await menuItem(fixture, 'Export…');
    await settle(fixture);
    expect(backend.exportProfile).toHaveBeenCalled();
  });
});

describe('TomlEditor', () => {
  it('edits text, shows line numbers, inserts tabs and jumps to errors', async () => {
    const { draft } = configure();
    await draft.load();
    const fixture = TestBed.createComponent(TomlEditor);
    await settle(fixture);
    const lines = MOCK_PROFILE_TOML.split('\n').length;
    expect(all(fixture, '.gutter div')).toHaveLength(lines);
    const ta = el<HTMLTextAreaElement>(fixture, 'textarea');
    typeInto(ta, 'name = "x"\n[oops');
    await settle(fixture);
    expect(all(fixture, '.gutter div')).toHaveLength(2);
    expect(el(fixture, '.gutter .err').textContent).toBe('2');
    ta.setSelectionRange(0, 0);
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }));
    expect(draft.text().startsWith('  name')).toBe(true);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '.problem .link').click();
    expect(ta.selectionStart).toBe(draft.text().indexOf('[oops'));
    ta.dispatchEvent(new Event('scroll'));
  });
});

describe('ProfileForm', () => {
  async function render() {
    const ctx = configure(new MockBackend({ latency: false, dataset: () => ({ sources: [], unrecognized: [], records: [] }) }));
    await ctx.draft.load();
    const fixture = TestBed.createComponent(ProfileForm);
    await settle(fixture);
    const go = async (label: string) => {
      all<HTMLButtonElement>(fixture, '.nav-item').find((b) => b.textContent?.includes(label))!.click();
      await settle(fixture);
    };
    return { ...ctx, fixture, go, m: () => ctx.draft.model() };
  }

  it('edits general, files and instance settings, and points to Settings ▸ Parsers for formats', async () => {
    const { fixture, go, m } = await render();
    typeInto(el<HTMLInputElement>(fixture, '.body input'), 'Renamed');
    expect(m().name).toBe('Renamed');

    await go('Files');
    buttonByText(fixture, '+ Add glob').click();
    expect(m().files?.include?.at(-1)).toBe('');
    expect(el(fixture, '[data-testid=profile-parsers-hint]').textContent).toContain('Formats are parsers');
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(true);
    buttonByText(fixture, 'Settings ▸ Parsers').click();
    expect(run).toHaveBeenCalledWith('parsers.manage');
    expect(all(fixture, '.nav-item').map((b) => b.textContent?.trim())).not.toContain('Parsing');


    await go('Instance');
    buttonByText(fixture, '+ Add field').click();
    expect(m().instance?.key).toEqual(['']);
  });

  it('manages extractors with live preview', async () => {
    const { fixture, go, m, backend } = await render();
    const preview = vi.spyOn(backend, 'previewExtractor');
    await go('Extractors');
    await wait(400);
    await settle(fixture);
    expect(preview).toHaveBeenCalled();
    expect(all(fixture, '.outputs .cap').map((c) => c.textContent)).toEqual(['cpu', 'ws', 'threads']);
    const ed = el(fixture, 'imt-extractor-editor');
    typeInto(ed.querySelector<HTMLInputElement>('.grid2 input')!, 'stats');
    expect(m().extract?.[0]?.name).toBe('stats');
    typeInto(at(ed.querySelectorAll<HTMLInputElement>('.grid2 input'), 1), '@exception');
    expect(m().extract?.[0]?.from).toBe('@exception');
    typeInto(ed.querySelector<HTMLTextAreaElement>('textarea')!, 'CPU (?P<cpu>[\\d.]+) (?P<extra>\\w+) (?P<x_unit>\\w+)');
    await settle(fixture);
    buttonByText(fixture, 'Detect from pattern').click();
    await settle(fixture);
    expect(Object.keys(m().extract?.[0]?.fields ?? {})).toEqual(['cpu', 'ws', 'threads', 'extra']);
    const typeSel = el<HTMLSelectElement>(fixture, '[aria-label="extra type"]');
    typeInto(typeSel, 'duration', 'change');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="extra unit"]'), 's');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="extra unit from"]'), 'x_unit');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="extra convert to"]'), 'ms');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="extra unit"]'), '');
    expect(m().extract?.[0]?.fields?.['extra']).toEqual({ type: 'duration', unit_from: 'x_unit', to: 'ms' });
    el<HTMLButtonElement>(fixture, '[aria-label="Remove output extra"]').click();
    expect(m().extract?.[0]?.fields?.['extra']).toBeUndefined();
    buttonByText(fixture, '+ Condition').click();
    expect(m().extract?.[0]?.when).toHaveLength(2);
    for (const b of [...ed.querySelectorAll<HTMLButtonElement>('[aria-label="Remove condition"]')]) b.click();
    expect(m().extract?.[0]?.when).toBeUndefined();

    buttonByText(fixture, '+ New extractor').click();
    await settle(fixture);
    expect(m().extract).toHaveLength(2);
    expect(el(fixture, '.list-item.on').textContent).toContain('extractor-2');
    at(all<HTMLButtonElement>(fixture, '.li-main'), 0).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Delete extractor stats"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Delete extractor extractor-2"]').click();
    await settle(fixture);
    expect(m().extract).toBeUndefined();
    expect(text(fixture)).toContain('Extractors pull values out of text');
  });

  it('renders preview highlights, values and errors', async () => {
    const ctx = configure();
    await settle();
    await ctx.draft.load();
    const f2 = TestBed.createComponent(ProfileForm);
    await settle(f2);
    all<HTMLButtonElement>(f2, '.nav-item').find((b) => b.textContent?.includes('Extractors'))!.click();
    await settle(f2);
    await wait(400);
    await settle(f2);
    expect(all(f2, '.sample .hl').length).toBeGreaterThan(0);
    expect(text(f2)).toContain('2 matches · 0 failed · 2 candidates');
    expect(all(f2, '.values .k').map((k) => k.textContent)).toEqual(['cpu', 'ws', 'threads']);
    vi.spyOn(ctx.backend, 'previewExtractor').mockRejectedValueOnce('preview broke');
    ctx.draft.update((mm) => (at(mm.extract!, 0).regex = 'CPU (?P<cpu>[\\d.]+)%'));
    await wait(400);
    await settle(f2);
    expect(el(f2, '.err').textContent).toBe('preview broke');
  });

  it('manages markers including reordering', async () => {
    const { fixture, go, m, draft } = await render();
    await go('Markers');
    expect(all(fixture, '.marker-card')).toHaveLength(6);
    const first = el(fixture, '.marker-card');
    typeInto(first.querySelector<HTMLInputElement>('.kind')!, 'fatal2');
    typeInto(at(first.querySelectorAll<HTMLInputElement>('.mk-row .input'), 1), '');
    typeInto(first.querySelector<HTMLInputElement>('input[type=color]')!, '#123456');
    first.querySelector<HTMLInputElement>('input[type=checkbox]')!.click();
    expect(m().marker?.[0]).toMatchObject({ kind: 'fatal2', color: '#123456', lifecycle: true });
    expect(m().marker?.[0]?.label).toBeUndefined();
    buttonByText(fixture, '+ New marker').click();
    expect(m().marker).toHaveLength(7);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Delete marker custom"]').click();
    expect(m().marker).toHaveLength(6);
    const form = fixture.componentInstance as unknown as { dropMarker(e: { previousIndex: number; currentIndex: number }): void };
    form.dropMarker({ previousIndex: 0, currentIndex: 2 });
    expect(m().marker?.[2]?.kind).toBe('fatal2');
    const textBefore = draft.text();
    form.dropMarker({ previousIndex: 1, currentIndex: 1 });
    expect(draft.text()).toBe(textBefore);
  });

  it('manages charts', async () => {
    const { fixture, go, m } = await render();
    await go('Charts');
    expect(all(fixture, '.chart-card')).toHaveLength(3);
    const card = at(all(fixture, '.chart-card'), 1);
    const inputs = card.querySelectorAll<HTMLInputElement>('.grid4 input');
    typeInto(at(inputs, 0), 'cpu2');
    typeInto(at(inputs, 1), '');
    typeInto(card.querySelector<HTMLSelectElement>('.grid4 select')!, 'max', 'change');
    expect(m().chart?.[1]).toMatchObject({ id: 'cpu2', y: 'max(cpu)' });
    expect(m().chart?.[1]?.title).toBeUndefined();
    await settle(fixture);
    typeInto(at(inputs, 2), 'ws');
    expect(m().chart?.[1]?.y).toBe('max(ws)');
    typeInto(at(inputs, 3), '');
    typeInto(at(card.querySelectorAll<HTMLSelectElement>('.grid4 select'), 1), 'bar', 'change');
    typeInto(at(inputs, 4), 'MB');
    expect(m().chart?.[1]).toMatchObject({ type: 'bar', unit: 'MB' });
    expect(m().chart?.[1]?.bucket).toBeUndefined();
    typeInto(card.querySelector<HTMLSelectElement>('.grid4 select')!, 'count', 'change');
    expect(m().chart?.[1]?.y).toBe('count()');
    buttonByText(fixture, '+ New chart').click();
    expect(m().chart).toHaveLength(4);
    await settle(fixture);
    at(all<HTMLButtonElement>(fixture, '.chart-card .del .btn'), 3).click();
    expect(m().chart).toHaveLength(3);
    for (let i = 0; i < 3; i++) {
      at(all<HTMLButtonElement>(fixture, '.chart-card .del .btn'), 0).click();
      await settle(fixture);
    }
    expect(m().chart).toBeUndefined();
    buttonByText(fixture, '+ New chart').click();
    expect(m().chart).toHaveLength(1);
  });
});
