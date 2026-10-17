import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { Facet } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { FACET_LIMIT, FacetFilterEvent, FacetPopover } from './facet-popover';
import { LiveFeed } from '../live/live-feed';

@Component({
  imports: [FacetPopover],
  template: `
    <imt-facet-popover
      [field]="field()"
      [top]="40"
      [left]="12"
      [numeric]="numeric()"
      (closed)="events.push('closed')"
      (groupBy)="events.push('group:' + $event)"
      (addColumn)="events.push('column')"
      (chart)="events.push('chart')"
      (filter)="filters.push($event)"
    />
  `,
})
class Host {
  readonly field = signal('MachineName');
  readonly numeric = signal(false);
  readonly events: string[] = [];
  readonly filters: FacetFilterEvent[] = [];
}

describe('FacetPopover', () => {
  async function render(backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    return { fixture, host: fixture.componentInstance, backend };
  }

  it('refreshes the counts in place when live tail asks (no flash back to loading)', async () => {
    const { fixture, backend } = await render();
    const spy = vi.spyOn(backend, 'queryFacet');
    const feed = TestBed.inject(LiveFeed);
    feed.facetVersion.set(1);
    await settle(fixture);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(all(fixture, '.value')).toHaveLength(2);
  });

  it('lists the values of the field in the current results', async () => {
    const { fixture, backend } = await render();
    const spy = vi.spyOn(backend, 'queryFacet');
    expect(all(fixture, '.value')).toHaveLength(2);
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toBe('2 values in current results');
    expect(text(fixture)).toContain('WEB01');
    expect(spy).not.toHaveBeenCalled();
  });

  it('asks the engine for at most the engine limit', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'queryFacet');
    await render(backend);
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ field: 'MachineName', limit: FACET_LIMIT }));
    expect(FACET_LIMIT).toBe(1000);
  });

  it('emits filters, group, column and chart actions', async () => {
    const { fixture, host } = await render();
    host.numeric.set(true);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName equals WEB01"]').click();
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName not equal WEB01"]').click();
    buttonByText(fixture, 'Group by').click();
    buttonByText(fixture, 'Add to group').click();
    buttonByText(fixture, 'Add column').click();
    buttonByText(fixture, 'Chart this field').click();
    expect(host.filters).toEqual([
      { op: 'eq', value: 'WEB01' },
      { op: 'ne', value: 'WEB01' },
    ]);
    expect(host.events).toEqual(['group:false', 'group:true', 'column', 'chart']);
  });

  it('hides the chart action for non-numeric fields', async () => {
    const { fixture } = await render();
    expect(all(fixture, 'button').some((b) => b.textContent?.trim() === 'Chart this field')).toBe(false);
  });

  it('closes on Escape', async () => {
    const { fixture, host } = await render();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await settle(fixture);
    expect(host.events).toContain('closed');
  });

  it('filters as you type and says when nothing matches', async () => {
    const { fixture } = await render();
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Search values"]'), 'web02');
    await settle(fixture);
    expect(all(fixture, '.value')).toHaveLength(1);
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toContain('1 match');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Search values"]'), 'zzz');
    await settle(fixture);
    expect(text(fixture)).toContain('No value matches the search.');
  });

  it('handles 10,000 distinct values: renders a window, searches the loaded ones, says what is shown', async () => {
    const { fixture } = await render(new MockBackend({ latency: false, dataset: () => ({ sources: [], unrecognized: [], records: [] }), syntheticFacetValues: 10_000 }));
    expect(all(fixture, '.value').length).toBeLessThan(25);
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toBe('10,000+ values in current results · top 1,000 shown');
    expect(el(fixture, '[role=list]').getAttribute('aria-label')).toBe('Values');
    expect(all(fixture, '[role=listitem]')[0].getAttribute('aria-setsize')).toBe('1000');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Search values"]'), '00042');
    await settle(fixture);
    expect(all(fixture, '.value')).toHaveLength(1);
    expect(text(fixture)).toContain('MachineName-value-00042');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Search values"]'), 'value-00');
    await settle(fixture);
    expect(all(fixture, '.value').length).toBeLessThan(25);
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toContain('1,000 match');
  });

  it('shows an inline error with Retry, then the values', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryFacet').mockRejectedValueOnce('facet failed');
    const { fixture } = await render(backend);
    expect(el(fixture, '[role=alert]').textContent).toContain('facet failed');
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toBe('unavailable');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(all(fixture, '.value')).toHaveLength(2);
  });

  it('reloads when the field changes and ignores a slower answer for the old field', async () => {
    const backend = tinyBackend();
    const slow: ((f: Facet) => void)[] = [];
    const real = backend.queryFacet.bind(backend);
    vi.spyOn(backend, 'queryFacet').mockImplementationOnce(async (r) => {
      const f = await real(r);
      return new Promise((resolve) => slow.push(() => resolve(f)));
    });
    const { fixture, host } = await render(backend);
    expect(text(fixture)).toContain('Loading');
    host.field.set('@level');
    await settle(fixture);
    slow[0]({} as Facet);
    await settle(fixture);
    expect(el(fixture, '[data-testid=facet-summary]').textContent).toContain('values in current results');
    expect(text(fixture)).toContain('Information');
  });

  it('shows (empty) for the empty value and a note when there are no values', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryFacet').mockResolvedValueOnce({ field: 'f', values: [{ value: '', count: 1 }], distinct: 1, total: 0 });
    const { fixture } = await render(backend);
    expect(text(fixture)).toContain('(empty)');
    expect(el<HTMLElement>(fixture, '.bar span').style.width).toBe('0%');
    const none = tinyBackend();
    vi.spyOn(none, 'queryFacet').mockResolvedValueOnce({ field: 'f', values: [], distinct: 0, total: 0 });
    TestBed.resetTestingModule();
    const again = await render(none);
    expect(text(again.fixture)).toContain('No values in the current results.');
  });
});
