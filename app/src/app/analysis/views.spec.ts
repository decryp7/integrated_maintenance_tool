import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { AnalysisBackend } from '../core/analysis-backend';
import { QlQuery } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { AnalysisState } from './analysis-state';
import { AnalysisTargets } from './analysis-targets';
import { AnomaliesView } from './anomalies-view';
import { CompareView } from './compare-view';
import { ExceptionFocus } from './exception-focus';
import { ExceptionsView } from './exceptions-view';
import { MomentView } from './moment-view';
import { PatternsView } from './patterns-view';
import { TimeCursor } from './time-cursor';
import { FakeContext, FIX, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';
import { at } from '../../testing/must';

async function mount<T>(view: Type<T>, backend: AnalysisBackend = stubBackend(), ctx = new FakeContext()) {
  TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend, context: ctx }) });
  const store = await loadWorkspace();
  const fixture = TestBed.createComponent(view);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture: fixture as ComponentFixture<T>, ctx, store, targets: TestBed.inject(AnalysisTargets), backend };
}

describe('AnalysisState', () => {
  it('shows error with retry, loading, empty and content states', async () => {
    const { fixture } = await mount(AnomaliesView, stubBackend({ queryAnomalies: () => Promise.reject(new Error('engine down')) }));
    expect(el(fixture, '[role=alert]').textContent).toContain('engine down');
    expect(text(fixture)).not.toContain('Loading');
    const state = TestBed.createComponent(AnalysisState);
    state.componentRef.setInput('loading', true);
    state.detectChanges();
    expect(text(state)).toContain('Loading…');
    state.componentRef.setInput('hasData', true);
    state.componentRef.setInput('empty', true);
    state.componentRef.setInput('emptyText', 'Nothing here');
    state.detectChanges();
    expect(text(state)).toContain('Nothing here');
    let retried = 0;
    state.componentInstance.retry.subscribe(() => retried++);
    state.componentRef.setInput('error', 'bad');
    state.detectChanges();
    buttonByText(state, 'Retry').click();
    expect(retried).toBe(1);
    // A failure reads as a sentence; the message is folded under Technical details (E5).
    expect(el(state, '[role=alert] strong').textContent).toBe("Couldn't finish this analysis.");
    expect(el(state, '[role=alert] details code').textContent).toBe('bad');
    // The engine says trying again as is cannot pass: no Retry, and its message (something to fix) is shown as is.
    state.componentRef.setInput('retryable', false);
    state.detectChanges();
    expect(text(state)).not.toContain('Retry');
    expect(el(state, '[role=alert] .msg').textContent).toBe('bad');
  });
});

describe('AnomaliesView', () => {
  it('lists findings with kind, instance, time range, message and score', async () => {
    const { fixture } = await mount(AnomaliesView);
    const rows = all<HTMLButtonElement>(fixture, 'li button');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain('Error spike');
    expect(rows[0]?.textContent).toContain('WEB01 · 3312');
    expect(rows[0]?.textContent).toContain('12 errors in 5 min');
    expect(rows[0]?.textContent).toContain('→');
    expect(rows[0]?.textContent).toContain('6.5');
    expect(rows[1]?.textContent).toContain('all instances');
    expect(rows[1]?.textContent).toContain('Crash');
    expect(text(fixture)).toContain('2 found');
  });

  it('has sortable column headers (K2)', async () => {
    const { fixture } = await mount(AnomaliesView);
    expect(all(fixture, '[data-testid=anomaly-head] .th').map((h) => h.textContent?.replace(/\s+/g, ' ').trim().replace(/[▲▼]/, '').trim())).toEqual(['Kind', 'Instance', 'Time', 'What', 'Score']);
    const first = () => all<HTMLButtonElement>(fixture, 'li button')[0]?.textContent ?? '';
    el<HTMLButtonElement>(fixture, '[data-testid=anomaly-sort-score]').click();
    fixture.detectChanges();
    expect(el(fixture, '[data-testid=anomaly-sort-score]').getAttribute('aria-pressed')).toBe('true');
    const byScore = first();
    el<HTMLButtonElement>(fixture, '[data-testid=anomaly-sort-score]').click();
    fixture.detectChanges();
    expect(first()).not.toBe(byScore);
  });

  it('click zooms to the range and jumps to the first sample', async () => {
    const { fixture, ctx } = await mount(AnomaliesView);
    at(all<HTMLButtonElement>(fixture, 'li button'), 0).click();
    await settle(fixture);
    expect(ctx.calls.map((c) => [c.fn, c.args])).toEqual([
      ['setQuery', [{ from: 1000, to: 5000 }]],
      ['jumpTo', [7]],
    ]);
    ctx.calls.length = 0;
    at(all<HTMLButtonElement>(fixture, 'li button'), 1).click();
    await settle(fixture);
    expect(ctx.calls.map((c) => c.fn)).toEqual(['setQuery']);
    expect(ctx.calls[0]?.args).toEqual([{ from: 9000, to: 9001 }]);
  });

  it('publishes the hovered time to the shared time cursor', async () => {
    const { fixture } = await mount(AnomaliesView);
    const cursor = TestBed.inject(TimeCursor);
    const row = at(all<HTMLButtonElement>(fixture, 'li button'), 0);
    row.dispatchEvent(new Event('mouseenter'));
    expect(cursor.hovered()).toBe(1000);
    row.dispatchEvent(new Event('mouseleave'));
    expect(cursor.hovered()).toBeNull();
    row.dispatchEvent(new Event('focus'));
    expect(cursor.hovered()).toBe(1000);
    row.dispatchEvent(new Event('blur'));
    expect(cursor.hovered()).toBeNull();
  });

  it('shows an empty message and refreshes on demand', async () => {
    const query = vi.fn(async () => []);
    const { fixture } = await mount(AnomaliesView, stubBackend({ queryAnomalies: query }));
    expect(text(fixture)).toContain('No anomalies detected');
    buttonByText(fixture, 'Refresh').click();
    await settle(fixture);
    expect(query).toHaveBeenCalledTimes(2);
  });

  it('retries a failed request', async () => {
    let fail = true;
    const { fixture } = await mount(AnomaliesView, stubBackend({ queryAnomalies: async () => (fail ? Promise.reject('boom') : FIX.anomalies) }));
    expect(el(fixture, '[role=alert]').textContent).toContain('boom');
    fail = false;
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, 'li button')).toHaveLength(2);
  });
});

describe('PatternsView', () => {
  it('ranks templates with level bars and counts', async () => {
    const { fixture } = await mount(PatternsView);
    const rows = all<HTMLButtonElement>(fixture, 'li .row');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain('120');
    expect(rows[0]?.textContent).toContain('Order {OrderId} placed');
    expect(el(fixture, '.bars').getAttribute('aria-label')).toBe('Information 100, Warning 15, Error 5');
    expect(rows[0]?.querySelectorAll('.bars span')).toHaveLength(3);
    // Bar length is relative to the most frequent pattern (120 → 90px, 30 → 23px).
    const bars = all<HTMLElement>(fixture, '.bars');
    expect([bars[0]?.style.width, bars[1]?.style.width]).toEqual(['90px', '23px']);
    expect(all(fixture, '.bars')[1]?.getAttribute('aria-label')).toBe('Information 30');
    expect(text(fixture)).toContain('2 templates');
  });

  it('filters by a Serilog template or a mined message pattern', async () => {
    const { fixture, ctx } = await mount(PatternsView);
    const rows = all<HTMLButtonElement>(fixture, 'li .row');
    at(rows, 0).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: '@template = "Order {OrderId} placed"' }]);
    at(rows, 1).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: '@message ~ "Took .+ ms"' }]);
  });

  it('ANDs the pattern with the query already applied', async () => {
    const { fixture, ctx } = await mount(PatternsView);
    TestBed.inject(WorkspaceStore).patchQuery({ ql: 'level >= warning or Ms > 5 | head 10' } as Partial<QlQuery>);
    at(all<HTMLButtonElement>(fixture, 'li .row'), 0).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: '(level >= warning or Ms > 5) and @template = "Order {OrderId} placed" | head 10' }]);
  });

  it('draws a sparkline for patterns with a trend and leaves the others blank', async () => {
    const { fixture } = await mount(PatternsView);
    const sparks = all<SVGElement>(fixture, '[data-testid=pattern-trend]');
    expect(sparks).toHaveLength(1);
    expect(sparks[0]?.querySelectorAll('rect')).toHaveLength(6);
    expect(sparks[0]?.getAttribute('aria-label')).toMatch(/^Occurrences over time, busiest around /);
    expect(all(fixture, '.spark-slot')).toHaveLength(1);
  });

  it('shows the values behind the placeholders and filters by one', async () => {
    const { fixture, ctx } = await mount(PatternsView);
    const buttons = all<HTMLButtonElement>(fixture, '.values-btn');
    expect(buttons).toHaveLength(1);
    expect(buttons[0]?.getAttribute('aria-expanded')).toBe('false');
    expect(all(fixture, '[data-testid=pattern-values]')).toHaveLength(0);
    at(buttons, 0).click();
    fixture.detectChanges();
    const panel = el(fixture, '[data-testid=pattern-values]');
    expect(buttons[0]?.getAttribute('aria-expanded')).toBe('true');
    expect(panel.textContent).toContain('Value 1 · 30 events');
    expect(panel.textContent).toContain('approximate');
    const values = all<HTMLButtonElement>(fixture, '.vrow .link');
    expect(values.map((v) => v.textContent?.trim())).toEqual(['12', '15']);
    expect(all(fixture, '.vrow .num').map((n) => n.textContent?.trim())).toEqual(['9', '4']);
    at(values, 0).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: '(@message ~ "Took .+ ms") and @message contains "12"' }]);
    at(buttons, 0).click();
    fixture.detectChanges();
    expect(all(fixture, '[data-testid=pattern-values]')).toHaveLength(0);
  });

  it('jumps to an example event of a pattern that has samples', async () => {
    const { fixture, ctx } = await mount(PatternsView);
    const examples = all<HTMLButtonElement>(fixture, 'li button.sample');
    expect(examples).toHaveLength(1);
    expect(examples[0]?.getAttribute('aria-label')).toBe('Show an example event of Order {OrderId} placed');
    at(examples, 0).click();
    await settle(fixture);
    expect(ctx.last('jumpTo')).toEqual([1]);
  });

  it('handles patterns without any level counts and empty results', async () => {
    const empty = await mount(PatternsView, stubBackend({ queryPatterns: async () => [] }));
    expect(text(empty.fixture)).toContain('No events to group');
    TestBed.resetTestingModule();
    const blank = await mount(PatternsView, stubBackend({ queryPatterns: async () => [{ ...FIX.patterns[0], levels: [0, 0, 0, 0, 0, 0] }] }));
    expect(el(blank.fixture, '[role=img]').getAttribute('aria-label')).toBe('no events');
  });
});

describe('ExceptionsView', () => {
  it('lists groups and expands instances and samples', async () => {
    const { fixture, ctx } = await mount(ExceptionsView);
    const heads = all<HTMLButtonElement>(fixture, '.head');
    expect(heads).toHaveLength(2);
    expect(heads[0]?.textContent).toContain('System.TimeoutException');
    expect(heads[0]?.textContent).toContain('Foo.Bar()');
    expect(heads[0]?.getAttribute('aria-expanded')).toBe('false');
    expect(all(fixture, '.detail')).toHaveLength(0);

    at(heads, 0).click();
    fixture.detectChanges();
    expect(heads[0]?.getAttribute('aria-expanded')).toBe('true');
    const detail = el(fixture, '.detail');
    expect(detail.textContent).toContain('WEB01');
    expect(detail.textContent).toContain('WEB02');
    const samples = all<HTMLButtonElement>(fixture, '.samples button');
    expect(samples.map((s) => s.textContent?.trim())).toEqual(['Event 11', 'Event 12']);
    at(samples, 0).click();
    await settle(fixture);
    expect(ctx.last('jumpTo')).toEqual([11]);

    at(heads, 0).click();
    fixture.detectChanges();
    expect(all(fixture, '.detail')).toHaveLength(0);
    at(heads, 1).click();
    fixture.detectChanges();
    expect(el(fixture, '.detail').textContent).toContain('—');
  });

  it('selects, expands and clears a group requested by fingerprint (the search palette); an unknown one is ignored', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend: stubBackend(), context: new FakeContext() }) });
    await loadWorkspace();
    const focus = TestBed.inject(ExceptionFocus);
    focus.request('fp2');
    const fixture = TestBed.createComponent(ExceptionsView);
    fixture.detectChanges();
    await settle(fixture, 6);
    const groups = all(fixture, '.group');
    expect(groups[1]?.classList.contains('selected')).toBe(true);
    expect(groups[0]?.classList.contains('selected')).toBe(false);
    expect(all(fixture, '.detail')).toHaveLength(1);
    expect(at(all(fixture, '.head'), 1).getAttribute('aria-current')).toBe('true');
    expect(focus.fingerprint()).toBeNull();

    focus.request('no-such-group');
    await settle(fixture, 2);
    expect(focus.fingerprint()).toBeNull();
    expect(all(fixture, '.detail')).toHaveLength(1);
  });

  it('says so when there are none', async () => {
    const { fixture } = await mount(ExceptionsView, stubBackend({ queryExceptions: async () => [] }));
    expect(text(fixture)).toContain('No exceptions');
  });
});

describe('MomentView', () => {
  it('asks for a selection when nothing is loaded', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend: stubBackend(), context: new FakeContext() }) });
    const fixture = TestBed.createComponent(MomentView);
    fixture.detectChanges();
    expect(text(fixture)).toContain('Select a time range');
  });

  it('renders summary, instances, metrics, users, activity and anomalies', async () => {
    const { fixture, targets } = await mount(MomentView);
    targets.setMoment({ from: 1000, to: 61000 });
    await settle(fixture, 6);
    const t = text(fixture);
    expect(t).toContain('3,000 events');
    expect(t).toContain('Information 2,900');
    expect(t).toContain('Warning 70');
    expect(t).toContain('Error 30');

    const cards = all(fixture, '.card');
    expect(cards).toHaveLength(2);
    expect(cards[0]?.textContent).toContain('2,000 events');
    expect(cards[0]?.textContent).toContain('25 errors');
    expect(cards[0]?.textContent).toContain('Failed to capture payment');
    expect(cards[0]?.textContent).toContain('Go to first error');
    expect(cards[1]?.textContent).not.toContain('Go to first error');

    const rows = all(fixture, 'tbody tr');
    expect(rows).toHaveLength(3);
    expect(rows[0]?.textContent).toContain('Cpu');
    expect(rows[0]?.textContent).toContain('+177.5%');
    expect(rows[0]?.textContent).toContain('▲');
    expect(rows[0]?.querySelector('.up')).toBeTruthy();
    expect(rows[1]?.textContent).toContain('−37.5%');
    expect(rows[1]?.textContent).toContain('▼');
    expect(rows[1]?.querySelector('.down')).toBeTruthy();
    expect(rows[2]?.textContent).toContain('—');
    expect(rows[2]?.querySelector('.flat')).toBeTruthy();

    expect(all(fixture, '.chips .chip').map((c) => c.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['ann 9', 'bob 2']);
    expect(all(fixture, '.activity .row')).toHaveLength(2);
    expect(all(fixture, '.activity .row')[1]?.textContent).toContain('—');
    expect(all(fixture, 'h3.sec').map((h) => h.textContent)).toEqual(['Instances', 'Metrics', 'Users', 'User activity', 'Anomalies in this window']);
  });

  it('click-throughs select records, filter by user and zoom', async () => {
    const { fixture, targets, ctx } = await mount(MomentView);
    targets.setMoment({ from: 1000, to: 61000 });
    await settle(fixture, 6);
    buttonByText(fixture, 'Go to first error').click();
    await settle(fixture);
    expect(ctx.last('jumpTo')).toEqual([77]);
    const chip = at(all<HTMLButtonElement>(fixture, '.chips .chip'), 0);
    expect(chip.title).toBe('Filter by UserName = ann');
    chip.click();
    expect(ctx.last('setQuery')).toEqual([{ ql: 'UserName = "ann"' }]);
    TestBed.inject(WorkspaceStore).patchQuery({ ql: 'level >= error' } as Partial<QlQuery>);
    chip.click();
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error and UserName = "ann"' }]);
    ctx.calls.length = 0;
    at(all<HTMLButtonElement>(fixture, '.activity .row'), 0).click();
    await settle(fixture);
    expect(ctx.last('jumpTo')).toEqual([5]);
    ctx.calls.length = 0;
    const anomalyRows = all<HTMLButtonElement>(fixture, 'h3.sec ~ ul.list:last-of-type .row');
    at(anomalyRows, 0).click();
    at(anomalyRows, 1).click();
    await settle(fixture);
    buttonByText(fixture, 'Zoom to this range').click();
    expect(ctx.last('setQuery')).toEqual([{ from: 1000, to: 61000 }]);
  });

  it('publishes the time of the hovered activity row', async () => {
    const { fixture, targets } = await mount(MomentView);
    targets.setMoment({ from: 1000, to: 61000 });
    await settle(fixture, 6);
    const row = at(all<HTMLButtonElement>(fixture, '.activity .row'), 0);
    row.dispatchEvent(new Event('mouseenter'));
    expect(TestBed.inject(TimeCursor).hovered()).toBe(2000);
    row.dispatchEvent(new Event('focus'));
    row.dispatchEvent(new Event('mouseleave'));
    expect(TestBed.inject(TimeCursor).hovered()).toBeNull();
    row.dispatchEvent(new Event('blur'));
  });

  it('follows the time cursor selection and shows empty sections', async () => {
    const empty = { ...FIX.moment, total: 0, levels: {}, instances: [], metrics: [], users: [], activity: [], anomalies: [] };
    const queryMoment = vi.fn(async () => empty);
    const { fixture } = await mount(MomentView, stubBackend({ queryMoment }));
    expect(queryMoment).toHaveBeenCalled();
    TestBed.inject(TimeCursor).select({ from: 5000, to: 9000 });
    await settle(fixture, 6);
    expect(queryMoment).toHaveBeenLastCalledWith(expect.anything(), 5000, 9000);
    const t = text(fixture);
    expect(t).toContain('No events in this window');
    expect(t).toContain('No numeric metrics');
    expect(t).toContain('No user information');
    expect(t).toContain('No user actions');
    expect(t).toContain('None.');
  });

  it('shows errors with retry', async () => {
    let fail = true;
    const { fixture } = await mount(MomentView, stubBackend({ queryMoment: async () => (fail ? Promise.reject('no moment') : FIX.moment) }));
    expect(el(fixture, '[role=alert]').textContent).toContain('no moment');
    fail = false;
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(text(fixture)).toContain('3,000 events');
  });
});

describe('CompareView', () => {
  it('asks for two windows first, then compares', async () => {
    const compare = vi.fn(async () => FIX.comparison);
    const { fixture, targets } = await mount(CompareView, stubBackend({ compareWindows: compare }));
    expect(text(fixture)).toContain('Pick two windows');
    expect(compare).not.toHaveBeenCalled();
    targets.setWindows({ from: 0, to: 60000 }, { from: 60000, to: 120000 });
    await settle(fixture, 6);
    expect(compare).toHaveBeenCalledWith(expect.objectContaining({ from: 0, to: 60000 }), expect.objectContaining({ from: 60000, to: 120000 }));
    const t = text(fixture);
    expect(t).toContain('Errors increased 12x');
    expect(t).toContain('Window A');
    expect(t).toContain('100 events');
    expect(t).toContain('300 events');
    expect(t).toContain('StatusCode');
    expect(t).toContain('A 1% · B 40%');
    expect(t).toContain('+150%');
    expect(t).toContain('−25%');
    expect(t).toContain('web03');
    expect(t).toContain('web01');
    expect(all(fixture, '.fill.a, .fill.b')).toHaveLength(4);
  });

  it('shows empty change sections', async () => {
    const calm = { ...FIX.comparison, changedFields: [], numericChanges: [], newValues: [], goneValues: [] };
    const { fixture, targets } = await mount(CompareView, stubBackend({ compareWindows: async () => calm }));
    targets.setWindows({ from: 0, to: 1 }, { from: 1, to: 2 });
    await settle(fixture, 6);
    const t = text(fixture);
    expect(t).toContain('No field distribution changed');
    expect(t).toContain('No numeric field changed');
    expect(t).toContain('Nothing new');
    expect(t).toContain('Nothing gone');
  });

  it('builds windows from the current range, a selected time and brushes', async () => {
    const { fixture, targets } = await mount(CompareView);
    const range = targets.currentRange()!;
    buttonByText(fixture, 'Current range vs previous').click();
    expect(targets.windowB()).toEqual(range);
    expect(targets.windowA()).toEqual({ from: range.from - (range.to - range.from), to: range.from });

    buttonByText(fixture, 'Before / after selected time').click();
    const mid = Math.round((range.from + range.to) / 2);
    expect(targets.windowA()).toEqual({ from: mid - 300_000, to: mid });
    expect(targets.windowB()).toEqual({ from: mid, to: mid + 300_000 });

    const select = el<HTMLSelectElement>(fixture, 'select');
    select.value = String(60_000);
    select.dispatchEvent(new Event('change'));
    TestBed.inject(TimeCursor).select({ from: 10_000_000, to: 10_100_000 });
    fixture.detectChanges();
    buttonByText(fixture, 'Before / after selected time').click();
    expect(targets.windowA()).toEqual({ from: 10_000_000 - 60_000, to: 10_000_000 });

    buttonByText(fixture, 'Set A from selection').click();
    buttonByText(fixture, 'Set B from selection').click();
    expect(targets.windowA()).toEqual({ from: 10_000_000, to: 10_100_000 });
    expect(targets.windowB()).toEqual({ from: 10_000_000, to: 10_100_000 });
  });

  it('does nothing without a workspace range and reports failures', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend: stubBackend({ compareWindows: () => Promise.reject('cmp failed') }), context: new FakeContext() }) });
    const fixture = TestBed.createComponent(CompareView);
    fixture.detectChanges();
    buttonByText(fixture, 'Current range vs previous').click();
    buttonByText(fixture, 'Before / after selected time').click();
    const targets = TestBed.inject(AnalysisTargets);
    expect(targets.windowA()).toBeNull();
    targets.setWindows({ from: 0, to: 1 }, { from: 1, to: 2 });
    await settle(fixture, 6);
    expect(el(fixture, '[role=alert]').textContent).toContain('cmp failed');
  });
});
