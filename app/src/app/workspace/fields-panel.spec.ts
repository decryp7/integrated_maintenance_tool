import { FIELDS_OPEN_KEY } from '../shell/sidebar-sections';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { Sidebar } from '../shell/sidebar';
import { FieldsPanel } from './fields-panel';
import { at } from '../../testing/must';

describe('Sidebar fields with the facet popover', () => {
  // The Fields section starts collapsed; these tests use it expanded.
  beforeEach(() => localStorage.setItem(FIELDS_OPEN_KEY, 'true'));
  afterEach(() => localStorage.removeItem(FIELDS_OPEN_KEY));
  async function render(backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock()] });
    const fixture = TestBed.createComponent(Sidebar);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store };
  }

  it('shows coverage and distinct counts, and 10k+ for capped fields', async () => {
    const { fixture } = await render();
    expect(all(fixture, '.field').length).toBeGreaterThan(3);
    const distinct = all(fixture, '.field .distinct').map((d) => d.textContent?.trim());
    expect(distinct.every((d) => /^[\d.]+k?\+?$/.test(d!) || d === '10k+')).toBe(true);
  });

  it('opens the popover next to the field, toggles it off, and replaces it when another field is chosen', async () => {
    const { fixture } = await render();
    const fields = all<HTMLButtonElement>(fixture, '.field');
    at(fields, 0).click();
    await settle(fixture);
    const pop = el<HTMLElement>(fixture, 'imt-facet-popover');
    expect(pop.style.left).not.toBe('');
    expect(Number.parseFloat(pop.style.top)).toBeGreaterThanOrEqual(40);
    at(fields, 0).click();
    await settle(fixture);
    expect(all(fixture, 'imt-facet-popover')).toHaveLength(0);
    at(fields, 1).click();
    await settle(fixture);
    expect(el(fixture, 'imt-facet-popover [data-testid=facet-summary]').textContent).toContain('values');
  });

  it('adds a column from the popover and closes it', async () => {
    const { fixture, store } = await render();
    at(all<HTMLButtonElement>(fixture, '.field'), 0).click();
    await settle(fixture);
    buttonByText(fixture, 'Add column').click();
    await settle(fixture);
    expect(store.extraColumns().length).toBe(1);
    expect(all(fixture, 'imt-facet-popover')).toHaveLength(0);
  });

  it('hides the Fields section in an empty workspace, and copes with a filter that matches nothing', async () => {
    const { fixture, store } = await render(new MockBackend({ latency: false, dataset: () => ({ sources: [], unrecognized: [], records: [] }) }));
    expect(all(fixture, '.field')).toHaveLength(0);
    expect(text(fixture)).toContain('No files open');
    expect(fixture.nativeElement.querySelector('imt-fields-panel')).toBeNull();
    TestBed.resetTestingModule();
    const loaded = await render();
    typeInto(el<HTMLInputElement>(loaded.fixture, 'input.filter'), 'zzz');
    await settle(loaded.fixture);
    expect(all(loaded.fixture, '.field')).toHaveLength(0);
    expect(store.summary()?.total).toBe(0);
  });

  it('starts collapsed when the user has not chosen, and remembers the choice', async () => {
    localStorage.removeItem(FIELDS_OPEN_KEY);
    const { fixture } = await render();
    const toggle = el<HTMLButtonElement>(fixture, '[data-testid=fields-toggle]');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(fixture.nativeElement.querySelector('input.filter')).toBeNull();
    toggle.click();
    await settle(fixture);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(all(fixture, '.field').length).toBeGreaterThan(3);
    expect(localStorage.getItem(FIELDS_OPEN_KEY)).toBe('true');
    toggle.click();
    await settle(fixture);
    expect(localStorage.getItem(FIELDS_OPEN_KEY)).toBe('false');
  });


  it('shows 0% coverage while no events exist', async () => {
    const { fixture, store } = await render();
    store.applySummary({ ...store.summary()!, total: 0 });
    await settle(fixture);
    expect(all<HTMLElement>(fixture, '.cov span').every((s) => s.style.width === '0%')).toBe(true);
  });
});

describe('FieldsPanel statistics action', () => {
  // The Fields section starts collapsed; these tests use it expanded.
  beforeEach(() => localStorage.setItem(FIELDS_OPEN_KEY, 'true'));
  afterEach(() => localStorage.removeItem(FIELDS_OPEN_KEY));
  it('offers an accessible Statistics… button per field that opens the field statistics view', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const fixture = TestBed.createComponent(FieldsPanel);
    await settle(fixture);
    const names = all(fixture, '.field .fname').map((n) => n.textContent?.trim());
    const buttons = all<HTMLButtonElement>(fixture, 'button.stats');
    expect(buttons).toHaveLength(names.length);
    expect(buttons[0]?.getAttribute('aria-label')).toBe(`Statistics for ${names[0]}`);
    expect(buttons[0]?.title).toBe('Statistics…');
    expect(buttons[0]?.type).toBe('button');

    // An open facet popover closes when statistics are requested.
    at(all<HTMLButtonElement>(fixture, '.field'), 1).click();
    await settle(fixture);
    expect(all(fixture, 'imt-facet-popover')).toHaveLength(1);
    at(buttons, 0).click();
    await settle(fixture);
    expect(TestBed.inject(AnalysisTargets).field()).toBe(names[0]);
    expect(dock.calls).toContain('show:field-stats');
    expect(all(fixture, 'imt-facet-popover')).toHaveLength(0);
  });

  it('charts a field from the popover in the metrics view', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const fixture = TestBed.createComponent(FieldsPanel);
    await settle(fixture);
    const numeric = TestBed.inject(WorkspaceStore).numericFields()[0];
    const index = all(fixture, '.field .fname').findIndex((n) => n.textContent?.trim() === numeric);
    at(all<HTMLButtonElement>(fixture, '.field'), index).click();
    await settle(fixture);
    const chart = all<HTMLButtonElement>(fixture, 'imt-facet-popover button').find((b) => /chart/i.test(b.textContent ?? ''));
    expect(chart).toBeDefined();
    chart!.click();
    await settle(fixture);
    expect(dock.calls).toContain('show:metrics');
    expect(all(fixture, 'imt-facet-popover')).toHaveLength(0);
  });
});
