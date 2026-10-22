import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, SEC, T0, text, typeInto } from '../../testing/helpers';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { AnalysisBackend } from '../core/analysis-backend';
import { ImtBackend } from '../core/backend';
import { HISTORY_STORE } from '../analysis/history-store';
import { MemoryHistory } from '../analysis/test-setup';
import { MockAnalysisBackend, recordsFrom } from '../analysis/mock/mock-analysis-backend';
import { inject } from '@angular/core';
import { COMPACT_WIDTH_PX, FilterBar } from './filter-bar';

describe('FilterBar', () => {
  async function render() {
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(ImtBackend))) },
        { provide: HISTORY_STORE, useValue: new MemoryHistory() },
      ],
    });
    const fixture = TestBed.createComponent(FilterBar);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store };
  }

  it('edits the search text, regex and case flags', async () => {
    const { fixture, store } = await render();
    const input = el<HTMLInputElement>(fixture, '[data-testid=query-field]');
    typeInto(input, 'timeout');
    await settle(fixture);
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
    expect(el(fixture, 'imt-query-field .box').classList.contains('invalid')).toBe(true);
    store.setTextInput('(?P<x>a)');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(el(fixture, 'imt-query-field .box').classList.contains('invalid')).toBe(false);
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

  it('shows each level chip as "matching/total" under the other filters, dimmed when none match (D5)', async () => {
    const { fixture, store } = await render();
    const chip = (code: string) => all<HTMLButtonElement>(fixture, '.lvl').find((b) => b.textContent?.includes(code))!;
    expect(el<HTMLInputElement>(fixture, '[data-testid=query-field]').placeholder).toBe('Search or query…');
    expect(chip('ERR').textContent).not.toContain('/');
    store.setTextInput('zzqx-nothing-says-this');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(chip('ERR').textContent).toMatch(/0\/1\b/);
    expect(chip('ERR').classList.contains('none')).toBe(true);
    expect(chip('ERR').title).toContain('0 of 1 match the other filters');
    // The chips' own selection does not change their counts.
    store.toggleLevel('Error');
    await settle(fixture);
    expect(chip('ERR').textContent).toMatch(/0\/1\b/);
    store.setTextInput('');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(chip('ERR').textContent).not.toContain('/');
    expect(chip('ERR').classList.contains('none')).toBe(false);
  });

  it('makes the level chips one tab stop; the arrow keys move between them (G1)', async () => {
    const { fixture } = await render();
    const chips = all<HTMLButtonElement>(fixture, '.lvl');
    const toolbar = el(fixture, '[role=toolbar][aria-label=Levels]');
    expect(chips.map((c) => c.tabIndex)).toEqual([0, -1, -1, -1, -1, -1]);
    const press = (key: string) => toolbar.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    at(chips, 0).focus();
    press('ArrowRight');
    await settle(fixture);
    expect(document.activeElement).toBe(at(chips, 1));
    expect(chips.map((c) => c.tabIndex)).toEqual([-1, 0, -1, -1, -1, -1]);
    press('ArrowLeft');
    press('ArrowLeft');
    await settle(fixture);
    expect(document.activeElement).toBe(at(chips, 5));
    press('Home');
    await settle(fixture);
    expect(document.activeElement).toBe(at(chips, 0));
    press('End');
    await settle(fixture);
    expect(document.activeElement).toBe(at(chips, 5));
    expect(press('x')).toBe(true);
  });

  it('edits the group-by in View options', async () => {
    const { fixture, store } = await render();
    const options = el<HTMLButtonElement>(fixture, '[data-testid=view-options]');
    expect(options.getAttribute('aria-expanded')).toBe('false');
    options.click();
    await settle(fixture);
    expect(options.getAttribute('aria-expanded')).toBe('true');
    expect(el(fixture, '[role=dialog]').getAttribute('aria-label')).toBe('View options');
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
    expect(text(fixture)).toContain('(1 min 30 s)');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove instance filter WEB01 · 100"]').click();
    expect(store.query().groups).toEqual([]);
    buttonByText(fixture, /14 Oct/).click();
    expect(store.hasRange()).toBe(false);
    store.setRange(T0, null);
    await settle(fixture);
    expect(text(fixture)).toContain('14 Oct 09:00:00 →');
    store.resetRange();

    el<HTMLButtonElement>(fixture, '[data-testid=view-options]').click();
    await settle(fixture);
    const switchFor = (name: RegExp) =>
      [...fixture.nativeElement.querySelectorAll('imt-switch')].find((s: HTMLElement) => name.test(s.textContent ?? ''))!.querySelector('input[role=switch]') as HTMLInputElement;
    expect(switchFor(/Marked only/).checked).toBe(false);
    switchFor(/Marked only/).click();
    expect(store.markedOnly()).toBe(true);
    switchFor(/Always show start/).click();
    expect(store.alwaysShowLifecycle()).toBe(false);
    store.setTimezone('utc');
    await settle(fixture);
    buttonByText(fixture, 'UTC').click();
    expect(store.timezone()).toBe('local');
    await settle(fixture);
    buttonByText(fixture, 'Clear all').click();
    expect(store.markedOnly()).toBe(false);
  });

  it('keeps one row: chips get a second row only while there are field or instance filters', async () => {
    const { fixture, store } = await render();
    expect(all(fixture, '.row')).toHaveLength(1);
    store.addFilter({ field: 'MachineName', op: 'eq', value: 'WEB01' });
    await settle(fixture);
    expect(all(fixture, '.row')).toHaveLength(2);
    expect(all(fixture, 'imt-switch')).toHaveLength(0);
  });

  it('focuses the search box on request', async () => {
    const { fixture, store } = await render();
    store.focusSearch();
    await settle(fixture);
    await new Promise((r) => setTimeout(r, 10));
    expect(document.activeElement).toBe(el(fixture, '[data-testid=query-field]'));
  });

  it('shows "All time" without data', async () => {
    const { fixture, store } = await render();
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('All time');
  });

  describe('narrow bar (H1)', () => {
    /** Reports the bar's width to the component's ResizeObserver. */
    function withWidth(width: number) {
      const original = globalThis.ResizeObserver;
      globalThis.ResizeObserver = class {
        constructor(private readonly cb: ResizeObserverCallback) {}
        observe(target: Element): void {
          this.cb([{ target, contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver);
        }
        unobserve(): void {}
        disconnect(): void {}
      } as unknown as typeof ResizeObserver;
      return () => (globalThis.ResizeObserver = original);
    }

    it('folds the range, time zone and Clear all into a ⋯ menu below the threshold', async () => {
      const restore = withWidth(COMPACT_WIDTH_PX - 1);
      try {
        const { fixture, store } = await render();
        store.setMarkedOnly(true);
        await settle(fixture);
        const hidden = (sel: string) => getComputedStyle(el(fixture, sel)).display === 'none';
        expect(hidden('.range')).toBe(true);
        expect(hidden('.tz')).toBe(true);
        const more = el<HTMLButtonElement>(fixture, '[data-testid=filter-more]');
        expect(more.getAttribute('aria-label')).toBe('More filter controls');
        more.click();
        await settle(fixture);
        const items = all<HTMLButtonElement>(fixture, '.more-pop [role=menuitem]').map((b) => b.textContent?.trim());
        expect(items.some((t) => t?.startsWith('Time zone:'))).toBe(true);
        expect(items).toContain('Clear all');
        all<HTMLButtonElement>(fixture, '.more-pop [role=menuitem]').find((b) => b.textContent?.trim() === 'Clear all')!.click();
        await settle(fixture);
        expect(store.markedOnly()).toBe(false);
        expect(all(fixture, '.more-pop')).toHaveLength(0);
        more.click();
        await settle(fixture);
        const tz = store.timezone();
        at(all<HTMLButtonElement>(fixture, '.more-pop [role=menuitem]'), 1).click();
        expect(store.timezone()).not.toBe(tz);
      } finally {
        restore();
      }
    });

    it('shows everything inline when the bar is wide enough, with no ⋯ menu', async () => {
      const restore = withWidth(COMPACT_WIDTH_PX + 200);
      try {
        const { fixture } = await render();
        expect(all(fixture, '[data-testid=filter-more]')).toHaveLength(0);
        expect(getComputedStyle(el(fixture, '.range')).display).not.toBe('none');
      } finally {
        restore();
      }
    });
  });
});
