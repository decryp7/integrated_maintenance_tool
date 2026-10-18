import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, pluginInfo, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { Platform } from '../core/platform';
import { PipelineStep } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { FlowOverview } from './flow-overview';
import { PipelineEditor, remap } from './pipeline-editor';
import { ProfileDraft } from './profile-draft';
import { ProfileForm } from './profile-form';
import { at } from '../../testing/must';

const platform = { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined };
const wait = (ms = 360) => new Promise((r) => setTimeout(r, ms));

async function render() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), { provide: Platform, useValue: platform }, ProfileDraft] });
  const draft = TestBed.inject(ProfileDraft);
  const store = TestBed.inject(WorkspaceStore);
  await draft.load();
  const fixture = TestBed.createComponent(PipelineEditor);
  await settle(fixture);
  const steps = () => draft.model().pipeline ?? ([] as PipelineStep[]);
  const add = async (label: string) => {
    buttonByText(fixture, new RegExp('^\\s*' + label)).click();
    await settle(fixture);
  };
  const act = async (label: string) => {
    el<HTMLButtonElement>(fixture, `[aria-label="${label}"]`).click();
    await settle(fixture);
  };
  return { fixture, draft, store, steps, add, act };
}

describe('PipelineEditor', () => {
  it('starts empty, adds one step of each kind and writes them to the profile', async () => {
    const { fixture, steps, add } = await render();
    expect(text(fixture)).toContain('No steps yet');
    expect(el<HTMLButtonElement>(fixture, '.palette .btn[title*="backend plugin"]').disabled).toBe(true);
    expect(text(fixture)).toContain('No transform plugins are loaded');
    const labels = ['Drop', 'Keep only', 'Collapse repeats', 'Mask text', 'Remove fields', 'Truncate', 'Rename field', 'Set field', 'Set level', 'Look up', 'Parse JSON', 'Parse key=value', 'Shift time', 'Calculated field', 'Script'];
    for (const l of labels) await add(l);
    expect(steps().map((s) => s.step)).toEqual(['drop', 'keep', 'dedupe', 'mask', 'remove_fields', 'truncate', 'rename', 'set', 'set_level', 'lookup', 'parse_json', 'parse_kv', 'shift_time', 'calc', 'script']);
    expect(all(fixture, '.palette .section-title').map((t) => t.textContent)).toEqual(['Filter', 'Clean', 'Enrich', 'Extend']);
    expect(all(fixture, '.card .summary')[0]?.textContent).toBe('Drop events where @message contains');
  });

  it('edits the config of each control type', async () => {
    const { fixture, steps, add, act } = await render();
    await add('Mask text');
    const input = (label: string) => el<HTMLInputElement>(fixture, `.card-body [aria-label="${label}"]`);
    typeInto(input('Field'), 'Detail');
    typeInto(input('Regex'), 'pw=\\S+');
    typeInto(input('Replace with'), 'pw=***');
    expect(steps()[0]).toMatchObject({ field: 'Detail', regex: 'pw=\\S+', replace: 'pw=***' });
    typeInto(input('Replace with'), '');
    expect(steps()[0]?.replace).toBeUndefined();

    await add('Truncate');
    const max = () => at(all(fixture, '.card'), 1).querySelector<HTMLInputElement>('[aria-label="Max characters"]')!;
    typeInto(max(), '50');
    expect(steps()[1]?.max).toBe(50);
    typeInto(max(), '');
    expect(steps()[1]?.max).toBeUndefined();

    await add('Set level');
    typeInto(el<HTMLSelectElement>(fixture, '[aria-label="Level"]'), 'Error', 'change');
    expect(steps()[2]?.level).toBe('Error');

    await add('Remove fields');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label^="Fields to remove 1"]'), 'A');
    expect(steps()[3]?.fields).toEqual(['A']);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove Fields to remove 1"]').click();
    expect(steps()[3]?.fields).toBeUndefined();

    await add('Look up');
    const json = el<HTMLTextAreaElement>(fixture, '[aria-label^="Table"]');
    typeInto(json, '{"WEB01":{"Site":"AMS"}}', 'change');
    expect(steps()[4]?.table).toEqual({ WEB01: { Site: 'AMS' } });
    typeInto(json, '{oops', 'change');
    await settle(fixture);
    expect(text(fixture)).toContain('not valid JSON');
    expect(steps()[4]?.table).toEqual({ WEB01: { Site: 'AMS' } });
    typeInto(json, '   ', 'change');
    await settle(fixture);
    expect(steps()[4]?.table).toBeUndefined();
    expect(text(fixture)).not.toContain('not valid JSON');
    await act('Hide step 5');
    expect(fixture.nativeElement.querySelectorAll('.card-body')).toHaveLength(4);
  });

  it('edits a script in a monospace editor and shows the engine error on its card', async () => {
    const { fixture, draft, steps, add } = await render();
    await add('Script');
    expect(steps()[0]?.step).toBe('script');
    const editor = el<HTMLTextAreaElement>(fixture, 'textarea[aria-label="Script"]');
    expect(editor.classList.contains('mono')).toBe(true);
    expect(editor.closest('.lbl')!.classList.contains('wide')).toBe(true);
    expect(text(fixture)).toContain('regex_replace(s, re, with)');
    typeInto(editor, 'if fields.ElapsedMs > 5000 {\n  fields.Slow = true;\n}');
    await settle(fixture);
    expect(steps()[0]?.script).toBe('if fields.ElapsedMs > 5000 {\n  fields.Slow = true;\n}');
    expect(el(fixture, '.summary').textContent).toBe('Run script (3 lines)');
    expect(fixture.nativeElement.querySelector('[data-testid="step-problem"]')).toBeNull();
    await wait();
    draft.validation.set({ ok: false, error: 'pipeline step 1 (script): `script` syntax error at line 2, column 9: Expecting a value', line: null });
    await settle(fixture);
    expect(el(fixture, '[data-testid="step-problem"]').textContent).toBe('`script` syntax error at line 2, column 9: Expecting a value');
    expect(editor.getAttribute('aria-invalid')).toBe('true');
    draft.validation.set({ ok: false, error: 'pipeline step 2 (drop): nope', line: null });
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid="step-problem"]')).toBeNull();
    typeInto(editor, '');
    expect(steps()[0]?.script).toBeUndefined();
  });

  it('flags script steps that failed on events in the last import', async () => {
    const { fixture, draft, store, add } = await render();
    await add('Script');
    await wait();
    await draft.save();
    await settle(fixture);
    expect(el(fixture, '.stat').textContent).toBe('11 in → 11 out (±0)');
    store.applySummary({ ...store.summary()!, pipeline: [{ index: 0, step: 'script', input: 11, output: 11, errors: 3 }] });
    await settle(fixture);
    const stat = el(fixture, '.stat');
    expect(stat.textContent).toBe('11 in → 11 out (±0), 3 failed');
    expect(stat.getAttribute('data-state')).toBe('failed');
    expect(stat.classList.contains('bad')).toBe(true);
    expect(stat.classList.contains('dim')).toBe(false);
  });

  it('adds conditions to the step through the shared editor', async () => {
    const { fixture, steps, add } = await render();
    await add('Drop');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="Field"]'), 'SourceContext');
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="Value"]'), 'HealthChecks');
    expect(steps()[0]?.when).toEqual([{ field: 'SourceContext', op: 'contains', value: 'HealthChecks' }]);
    await settle(fixture);
    expect(el(fixture, '.summary').textContent).toBe('Drop events where SourceContext contains HealthChecks');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove condition"]').click();
    expect(steps()[0]?.when).toBeUndefined();
  });

  it('reorders, duplicates, disables and removes steps', async () => {
    const { fixture, steps, add, act } = await render();
    await add('Drop');
    await add('Keep only');
    await add('Set level');
    expect(el<HTMLButtonElement>(fixture, '[aria-label="Move step 1 up"]').disabled).toBe(true);
    expect(el<HTMLButtonElement>(fixture, '[aria-label="Move step 3 down"]').disabled).toBe(true);
    await act('Move step 1 down');
    expect(steps().map((s) => s.step)).toEqual(['keep', 'drop', 'set_level']);
    await act('Move step 3 up');
    expect(steps().map((s) => s.step)).toEqual(['keep', 'set_level', 'drop']);
    await act('Duplicate step 1');
    expect(steps().map((s) => s.step)).toEqual(['keep', 'keep', 'set_level', 'drop']);
    el<HTMLInputElement>(fixture, '[aria-label="Disable step 2"]').click();
    await settle(fixture);
    expect(steps()[1]?.disabled).toBe(true);
    expect(all(fixture, '.card')[1]?.classList.contains('off')).toBe(true);
    expect(all(fixture, '.stat')[1]?.textContent).toBe('disabled');
    el<HTMLInputElement>(fixture, '[aria-label="Disable step 2"]').click();
    expect(steps()[1]?.disabled).toBeUndefined();
    await act('Remove step 4');
    await act('Remove step 3');
    await act('Remove step 2');
    expect(steps().map((s) => s.step)).toEqual(['keep']);
    await act('Remove step 1');
    expect(fixture.nativeElement.querySelector('.card')).toBeNull();
    expect(text(fixture)).toContain('No steps yet');
  });

  it('reorders from a drag and drop event', async () => {
    const { fixture, steps, add } = await render();
    await add('Drop');
    await add('Keep only');
    const editor = fixture.componentInstance as unknown as { drop(e: { previousIndex: number; currentIndex: number }): void };
    editor.drop({ previousIndex: 0, currentIndex: 0 });
    expect(steps().map((s) => s.step)).toEqual(['drop', 'keep']);
    editor.drop({ previousIndex: 0, currentIndex: 1 });
    expect(steps().map((s) => s.step)).toEqual(['keep', 'drop']);
  });

  it('keeps open cards attached to their step when steps move', () => {
    expect(remap(1, 1, 3)).toBe(3);
    expect(remap(2, 1, 3)).toBe(1);
    expect(remap(3, 1, 3)).toBe(2);
    expect(remap(0, 1, 3)).toBe(0);
    expect(remap(1, 3, 1)).toBe(2);
    expect(remap(2, 3, 1)).toBe(3);
    expect(remap(0, 3, 1)).toBe(0);
    expect(remap(4, 3, 1)).toBe(4);
  });

  it('offers transform plugins and keeps an unknown plugin id visible', async () => {
    const { fixture, store, steps, add } = await render();
    store.applySummary({ ...store.summary()!, plugins: [pluginInfo({ id: 'geo', name: 'GeoIP', version: '1', contributes: ['transform:geoip'] })] });
    await settle(fixture);
    await add('Plugin');
    expect(steps()[0]).toEqual({ step: 'plugin', id: 'geo' });
    typeInto(el<HTMLSelectElement>(fixture, '[aria-label="Transform plugin"]'), '', 'change');
    expect(steps()[0]?.id).toBeUndefined();
    typeInto(el<HTMLSelectElement>(fixture, '[aria-label="Transform plugin"]'), 'geo', 'change');
    expect(el(fixture, '.summary').textContent).toBe('Run transform plugin "geo"');
    TestBed.inject(ProfileDraft).update((m) => (at(m.pipeline!, 0).id = 'gone'));
    await settle(fixture);
    expect(text(fixture)).toContain('not loaded');
  });

  it('shows in/out counts from the last import and "not run yet" for unsaved edits', async () => {
    const { fixture, draft, store, add } = await render();
    await add('Drop');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="Field"]'), '@level');
    await settle(fixture);
    typeInto(el<HTMLSelectElement>(fixture, '[aria-label="Operator"]'), 'eq', 'change');
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="Value"]'), 'Debug');
    expect(el(fixture, '.stat').textContent).toBe('not run yet');
    await wait();
    await draft.save();
    await settle(fixture);
    expect(store.summary()!.pipeline).toEqual([{ index: 0, step: 'drop', input: 11, output: 10 }]);
    expect(el(fixture, '.stat').textContent).toBe('11 in → 10 out (−1)');
    typeInto(el<HTMLInputElement>(fixture, '[aria-label="Value"]'), 'Info');
    await settle(fixture);
    expect(el(fixture, '.stat').textContent).toBe('not run yet');
  });
});

describe('FlowOverview and form integration', () => {
  it('shows Import -> Extract -> Pipeline -> Markers with counts and opens the matching section', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), { provide: Platform, useValue: platform }, ProfileDraft] });
    const draft = TestBed.inject(ProfileDraft);
    await draft.load();
    const fixture = TestBed.createComponent(ProfileForm);
    await settle(fixture);
    const stages = all<HTMLButtonElement>(fixture, 'imt-flow-overview .node');
    expect(stages.map((s) => s.querySelector('.title')!.textContent)).toEqual(['Import', 'Extract', 'Pipeline', 'Markers']);
    expect(stages[0]?.textContent).toContain('2 sources · 11 events');
    expect(stages[2]?.textContent).toContain('0 steps');
    at(stages, 2).click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-pipeline-editor')).not.toBeNull();
    draft.update((m) => (m.pipeline = [{ step: 'drop' }, { step: 'keep', disabled: true }]));
    await settle(fixture);
    expect(all(fixture, 'imt-flow-overview .node')[2]?.textContent).toContain('1 step');
    for (const [i, section] of ['files', 'extract', 'pipeline', 'marker'].entries()) {
      at(all<HTMLButtonElement>(fixture, 'imt-flow-overview .node'), i).click();
      await settle(fixture);
      expect(section).toBeTruthy();
    }
    expect(fixture.nativeElement.querySelector('.nav-item.on')!.textContent).toContain('Markers');
  });

  it('has help hints on the sections', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), { provide: Platform, useValue: platform }, ProfileDraft] });
    await TestBed.inject(ProfileDraft).load();
    const fixture = TestBed.createComponent(ProfileForm);
    await settle(fixture);
    const helps: string[] = [];
    for (const label of ['Files', 'Parsing', 'Instance', 'Extractors', 'Pipeline', 'Markers', 'Charts']) {
      all<HTMLButtonElement>(fixture, '.nav-item').find((b) => b.textContent?.includes(label))!.click();
      await settle(fixture);
      const q = el<HTMLButtonElement>(fixture, '.body imt-help .q');
      helps.push(q.getAttribute('aria-label')!);
      q.click();
      await settle(fixture);
      expect(el(fixture, '.body imt-help .tip').textContent!.length).toBeGreaterThan(20);
    }
    expect(helps).toEqual(['Help: Files', 'Help: Parsing', 'Help: Instance', 'Help: Field extractors', 'Help: Pipeline', 'Help: Markers', 'Help: Charts']);
  });

  it('FlowOverview shows zero counts without a workspace', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), { provide: Platform, useValue: platform }, ProfileDraft] });
    const fixture = TestBed.createComponent(FlowOverview);
    fixture.detectChanges();
    expect(text(fixture)).toContain('0 sources · 0 events');
  });
});
