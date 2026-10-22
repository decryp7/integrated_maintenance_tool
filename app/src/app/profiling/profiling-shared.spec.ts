import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../testing/helpers';
import { CapturesStore } from './captures.store';
import { DEFAULT_FRAMEWORK_PATTERNS, ProfileViewState } from './profile-view-state';
import { CaptureNotes, MeasurementLegend, MeasurementTag, nextTagId, NoCapture, ProfileBar } from './profiling-shared';
import { openDemoCaptures, setupProfiling } from './testing/profiling-test-support';

async function mountBar(open = true, showMetric = true) {
  const doubles = setupProfiling();
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<ProfileBar> = TestBed.createComponent(ProfileBar);
  fixture.componentRef.setInput('showMetric', showMetric);
  fixture.detectChanges();
  await settle(fixture, 3);
  return { fixture, doubles, state: TestBed.inject(ProfileViewState) };
}

const sel = (f: ComponentFixture<unknown>, label: string) => el<HTMLSelectElement>(f, `select[aria-label="${label}"]`);

describe('MeasurementTag', () => {
  it('renders ≈ with a title and screen-reader text for sampled values', () => {
    setupProfiling();
    const f = TestBed.createComponent(MeasurementTag);
    f.componentRef.setInput('m', 'sampled');
    f.detectChanges();
    const m = el(f, '.m');
    expect(m.textContent).toBe('≈');
    expect(m.getAttribute('data-measurement')).toBe('sampled');
    expect(m.getAttribute('title')).toContain('statistical estimate');
    expect(m.getAttribute('aria-hidden')).toBe('true');
    expect(m.classList).toContain('sampled');
    expect(el(f, '.sr').textContent).toBe('(Sampled, estimate)');
  });

  it('renders = for exact values', () => {
    setupProfiling();
    const f = TestBed.createComponent(MeasurementTag);
    f.componentRef.setInput('m', 'exact');
    f.detectChanges();
    expect(el(f, '.m').textContent).toBe('=');
    expect(el(f, '.m').classList).not.toContain('sampled');
    expect(el(f, '.m').getAttribute('title')).toContain('every occurrence was recorded');
    expect(el(f, '.sr').textContent).toBe('(Exact)');
  });
});

describe('MeasurementLegend', () => {
  it('explains both tags', () => {
    setupProfiling();
    const f = TestBed.createComponent(MeasurementLegend);
    f.detectChanges();
    expect(text(f)).toContain('sampled estimate');
    expect(text(f)).toContain('exact');
    expect(all(f, '[data-measurement]').map((x) => x.getAttribute('data-measurement'))).toEqual(['sampled', 'exact']);
  });
});

describe('CaptureNotes', () => {
  it('lists notes and renders nothing without notes', () => {
    setupProfiling();
    const f = TestBed.createComponent(CaptureNotes);
    f.detectChanges();
    expect(all(f, '[role=note]')).toHaveLength(0);
    f.componentRef.setInput('notes', ['one', 'two']);
    f.componentRef.setInput('name', 'cap.json');
    f.detectChanges();
    expect(el(f, '[role=note]').getAttribute('aria-label')).toBe('Import notes for cap.json');
    expect(all(f, 'li').map((x) => x.textContent)).toEqual(['one', 'two']);
  });
});

describe('NoCapture', () => {
  it('shows the text and opens captures through the store', async () => {
    setupProfiling();
    const f = TestBed.createComponent(NoCapture);
    f.componentRef.setInput('text', 'Nothing yet');
    f.detectChanges();
    expect(text(f)).toContain('Nothing yet');
    buttonByText(f, 'Open performance capture…').click();
    await settle(f, 6);
    expect(TestBed.inject(CapturesStore).captures().length).toBe(5);
  });

  it('has a default text, shows the open error and disables the button while loading', async () => {
    const d = setupProfiling({ options: { latencyMs: 30 } });
    d.picker.paths = ['/tmp/bad.txt'];
    const f = TestBed.createComponent(NoCapture);
    f.detectChanges();
    expect(text(f)).toContain('Open a performance capture to see this view.');
    buttonByText(f, 'Open performance capture…').click();
    await new Promise((r) => setTimeout(r, 5));
    f.detectChanges();
    expect(buttonByText(f, 'Open performance capture…').disabled).toBe(true);
    await new Promise((r) => setTimeout(r, 150));
    await settle(f, 6);
    expect(el(f, '[role=alert]').textContent).toContain('bad.txt');
    expect(buttonByText(f, 'Open performance capture…').disabled).toBe(false);
  });
});

describe('nextTagId', () => {
  it('returns unique ids with the prefix', () => {
    const a = nextTagId('x');
    const b = nextTagId('x');
    expect(a).toMatch(/^x-\d+$/);
    expect(a).not.toBe(b);
  });
});

describe('ProfileBar', () => {
  it('renders nothing without an active profile', async () => {
    const { fixture } = await mountBar(false);
    expect(all(fixture, '[role=toolbar]')).toHaveLength(0);
  });

  it('switches the active profile with the profile select', async () => {
    const { fixture, state } = await mountBar();
    const select = sel(fixture, 'Active profile');
    expect(select.options.length).toBe(2);
    expect(select.value).toBe(String(state.profileId()));
    typeInto(select, '2', 'change');
    fixture.detectChanges();
    expect(state.profileId()).toBe(2);
    expect(sel(fixture, 'Active profile').selectedOptions[0]?.textContent).toContain('regression');
  });

  it('picks the metric and shows its measurement tag', async () => {
    const { fixture, state } = await mountBar();
    const metric = sel(fixture, 'Metric');
    expect(metric.selectedOptions[0]?.textContent).toMatch(/\((sampled|exact)\)/);
    expect(all(fixture, '[data-measurement]').length).toBe(1);
    typeInto(metric, 'samples', 'change');
    fixture.detectChanges();
    expect(state.metric()).toBe('samples');
    expect(sel(fixture, 'Metric').value).toBe('samples');
    expect(el(fixture, '[data-measurement]').getAttribute('data-measurement')).toBe('sampled');
  });

  it('hides the metric when showMetric is false', async () => {
    const { fixture } = await mountBar(true, false);
    expect(all(fixture, 'select[aria-label=Metric]')).toHaveLength(0);
    expect(all(fixture, '[data-measurement]')).toHaveLength(0);
  });

  it('shows the range pill and clears it', async () => {
    const { fixture, state } = await mountBar();
    expect(all(fixture, '[data-testid=range-pill]')).toHaveLength(0);
    state.setRange({ fromNs: 1_000_000, toNs: 5_000_000 });
    fixture.detectChanges();
    expect(el(fixture, '[data-testid=range-pill]').textContent).toMatch(/Range .+ – .+/);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Clear time range"]').click();
    fixture.detectChanges();
    expect(state.range()).toBeNull();
    expect(all(fixture, '[data-testid=range-pill]')).toHaveLength(0);
  });

  it('shows the threads pill (singular and plural) and shows all threads again', async () => {
    const { fixture, state } = await mountBar();
    state.setThreads([1]);
    fixture.detectChanges();
    expect(text(fixture)).toContain('1 thread');
    expect(text(fixture)).not.toContain('1 threads');
    state.setThreads([1, 2]);
    fixture.detectChanges();
    expect(text(fixture)).toContain('2 threads');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Show all threads"]').click();
    fixture.detectChanges();
    expect(state.threads()).toBeNull();
    expect(all(fixture, 'button[aria-label="Show all threads"]')).toHaveLength(0);
  });

  it('toggles just my code', async () => {
    const { fixture, state } = await mountBar();
    const box = el<HTMLInputElement>(fixture, 'label.inline input[type=checkbox]');
    expect(box.checked).toBe(false);
    box.click();
    fixture.detectChanges();
    expect(state.justMyCode()).toBe(true);
    expect(el<HTMLInputElement>(fixture, 'label.inline input[type=checkbox]').checked).toBe(true);
    box.click();
    expect(state.justMyCode()).toBe(false);
  });

  it('edits framework patterns: opens, applies cleaned patterns and enables just my code, closes', async () => {
    const { fixture, state } = await mountBar();
    const toggle = buttonByText(fixture, 'Framework patterns…');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(all(fixture, '[role=group]')).toHaveLength(0);
    toggle.click();
    fixture.detectChanges();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    const editorId = toggle.getAttribute('aria-controls')!;
    expect(editorId).toMatch(/^jmc-editor-\d+$/);
    expect(el(fixture, '[role=group]').id).toBe(editorId);
    const area = el<HTMLTextAreaElement>(fixture, 'textarea[aria-label="Framework patterns"]');
    expect(area.value).toBe(DEFAULT_FRAMEWORK_PATTERNS.join('\n'));
    area.value = '  Acme.*  \n\nAcme.*\nVendor.*\n';
    buttonByText(fixture, 'Apply patterns').click();
    fixture.detectChanges();
    expect(state.frameworkPatterns()).toEqual(['Acme.*', 'Vendor.*']);
    expect(state.justMyCode()).toBe(true);
    // use-frame-flags checkbox
    const flags = el<HTMLInputElement>(fixture, '[role=group] input[type=checkbox]');
    expect(flags.checked).toBe(true);
    flags.click();
    expect(state.useFrameFlags()).toBe(false);
    buttonByText(fixture, 'Close').click();
    fixture.detectChanges();
    expect(all(fixture, '[role=group]')).toHaveLength(0);
    expect(buttonByText(fixture, 'Framework patterns…').getAttribute('aria-expanded')).toBe('false');
  });

  it('sets the recursion folding', async () => {
    const { fixture, state } = await mountBar();
    expect([...sel(fixture, 'Recursion folding').options].map((o) => o.textContent)).toEqual(['No folding', 'Fold direct recursion', 'Fold all recursion']);
    typeInto(sel(fixture, 'Recursion folding'), 'direct', 'change');
    expect(state.recursion()).toBe('direct');
    fixture.detectChanges();
    expect(sel(fixture, 'Recursion folding').value).toBe('direct');
  });
});
