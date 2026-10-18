import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, SEC, T0, text, typeInto } from '../../testing/helpers';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { FilterBar } from './filter-bar';

describe('FilterBar', () => {
  async function render() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(FilterBar);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store };
  }

  it('edits the search text, regex and case flags', async () => {
    const { fixture, store } = await render();
    const input = el<HTMLInputElement>(fixture, '.search input');
    typeInto(input, 'timeout');
    expect(store.textInput()).toBe('timeout');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(store.textInput()).toBe('');
    const toggles = all<HTMLButtonElement>(fixture, '.toggle');
    const regex = at(toggles, 0);
    const cs = at(toggles, 1);
    regex.click();
    cs.click();
    expect(store.query()).toMatchObject({ regex: true, caseSensitive: true });
    await settle(fixture);
    expect(input.placeholder).toBe('Regular expression');
  });

  it('flags invalid regexes', async () => {
    const { fixture, store } = await render();
    store.setRegex(true);
    store.setTextInput('(');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(el(fixture, '.search').classList.contains('invalid')).toBe(true);
    store.setTextInput('(?P<x>a)');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(el(fixture, '.search').classList.contains('invalid')).toBe(false);
  });

  it('toggles levels and shows counts', async () => {
    const { fixture, store } = await render();
    const err = all<HTMLButtonElement>(fixture, '.lvl').find((b) => b.textContent?.includes('ERR'))!;
    expect(err.textContent).toContain('1');
    err.click();
    expect(store.query().levels).toEqual(['Error']);
    await settle(fixture);
    expect(err.getAttribute('aria-pressed')).toBe('true');
  });

  it('edits the group-by', async () => {
    const { fixture, store } = await render();
    buttonByText(fixture, /Group by/).click();
    await settle(fixture);
    all<HTMLButtonElement>(fixture, '.opt').find((b) => b.textContent?.includes('Application'))!.click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName', 'ProcessId', 'Application']);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '.chosen .x').click();
    expect(store.effectiveGroupBy()).toEqual(['ProcessId', 'Application']);
    buttonByText(fixture, 'Use profile default').click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName', 'ProcessId']);
  });

  it('adds and removes field filters', async () => {
    const { fixture, store } = await render();
    buttonByText(fixture, '+ Add filter').click();
    await settle(fixture);
    const form = el<HTMLFormElement>(fixture, '.add-pop');
    typeInto(form.querySelector('input')!, 'MachineName');
    typeInto(form.querySelectorAll('input')[1] as HTMLInputElement, 'WEB02');
    form.dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(store.query().filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB02' }]);
    expect(text(fixture)).toContain('MachineName=WEB02');

    buttonByText(fixture, '+ Add filter').click();
    await settle(fixture);
    const form2 = el<HTMLFormElement>(fixture, '.add-pop');
    typeInto(form2.querySelector('input')!, 'CorrelationId');
    typeInto(form2.querySelector('select')!, 'exists', 'change');
    await settle(fixture);
    expect(form2.querySelectorAll('input')).toHaveLength(1);
    form2.dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(store.query().filters[1]).toEqual({ field: 'CorrelationId', op: 'exists' });
    expect(text(fixture)).toContain('CorrelationIdexists');

    buttonByText(fixture, '+ Add filter').click();
    await settle(fixture);
    el<HTMLFormElement>(fixture, '.add-pop').dispatchEvent(new Event('submit'));
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture);
    expect(store.query().filters).toHaveLength(2);

    el<HTMLButtonElement>(fixture, '[aria-label="Remove filter MachineName"]').click();
    expect(store.query().filters).toHaveLength(1);
  });

  it('shows instance filters, range and switches', async () => {
    const { fixture, store } = await render();
    store.toggleGroup('WEB01 · 100');
    store.setRange(T0, T0 + 90 * SEC);
    await settle(fixture);
    expect(text(fixture)).toContain('instanceWEB01 · 100');
    expect(text(fixture)).toContain('(1.5 min)');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove instance filter WEB01 · 100"]').click();
    expect(store.query().groups).toEqual([]);
    buttonByText(fixture, /14 Oct/).click();
    expect(store.hasRange()).toBe(false);
    store.setRange(T0, null);
    await settle(fixture);
    expect(text(fixture)).toContain('14 Oct 09:00:00 →');
    store.resetRange();

    const switchFor = (name: RegExp) =>
      [...fixture.nativeElement.querySelectorAll('imt-switch')].find((s: HTMLElement) => name.test(s.textContent ?? ''))!.querySelector('input[role=switch]') as HTMLInputElement;
    expect(switchFor(/Marked only/).checked).toBe(false);
    switchFor(/Marked only/).click();
    expect(store.markedOnly()).toBe(true);
    switchFor(/Always show start/).click();
    expect(store.alwaysShowLifecycle()).toBe(false);
    buttonByText(fixture, 'UTC').click();
    expect(store.timezone()).toBe('local');
    await settle(fixture);
    buttonByText(fixture, 'Clear all').click();
    expect(store.markedOnly()).toBe(false);
  });

  it('focuses the search box on request', async () => {
    const { fixture, store } = await render();
    store.focusSearch();
    await settle(fixture);
    await new Promise((r) => setTimeout(r, 10));
    expect(document.activeElement).toBe(el(fixture, '.search input'));
  });

  it('shows "All time" without data', async () => {
    const { fixture, store } = await render();
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('All time');
  });
});
