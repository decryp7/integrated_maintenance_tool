import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { buttonByText, provideTinyBackend, query, settle, text } from '../../testing/helpers';
import { AnalysisState } from '../analysis/analysis-state';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { emptyCause, isFiltered, NextSteps } from './next-steps';

describe('next steps for empty and failed views', () => {
  it('suggests opening files first, then clearing filters, then the whole time range', () => {
    expect(emptyCause(false, query({ text: 'x' }))).toBe('no-files');
    expect(emptyCause(true, query({ levels: ['Error'] }))).toBe('filtered');
    expect(emptyCause(true, query({ from: 1 }))).toBe('no-range');
    expect(emptyCause(true, query({ to: 1 }))).toBe('no-range');
    expect(emptyCause(true, query())).toBe('none');
    expect(isFiltered({ ...query(), ql: 'level >= error' } as never)).toBe(true);
    expect(isFiltered(query({ markers: ['crash'] }))).toBe(true);
    expect(isFiltered(query())).toBe(false);
  });

  async function setup(demo = true) {
    const run = vi.fn();
    TestBed.configureTestingModule({
      providers: [...provideTinyBackend(), { provide: DEBRIEF_COMMANDS, multi: true, useValue: demo ? [{ id: 'help.demo', title: 'Try the demo', run }] : [] }],
    });
    const fixture = TestBed.createComponent(NextSteps);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store, run };
  }

  it('offers Clear all filters and Show the whole time range, which act on the store', async () => {
    const { fixture, store } = await setup();
    expect(text(fixture)).toBe('');
    store.toggleLevel('Error');
    await settle(fixture);
    buttonByText(fixture, /Clear all filters/).click();
    expect(store.query().levels).toEqual([]);
    store.setRange(1, 2);
    await settle(fixture);
    buttonByText(fixture, /Show the whole time range/).click();
    expect(store.query().from).toBeNull();
  });

  it('offers Open and the demo when nothing is open', async () => {
    const { fixture, store, run } = await setup();
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('Nothing is open yet.');
    const pick = vi.spyOn(store, 'pickAndOpen').mockResolvedValue();
    buttonByText(fixture, /Open folder/).click();
    buttonByText(fixture, /Open files/).click();
    expect(pick.mock.calls).toEqual([[true], [false]]);
    buttonByText(fixture, 'Try the demo').click();
    await settle(fixture);
    expect(run).toHaveBeenCalled();
    fixture.componentRef.setInput('hideOpen', true);
    await settle(fixture);
    expect(text(fixture)).toBe('');
  });

  it('leaves the demo out when it is not available', async () => {
    const { fixture, store } = await setup(false);
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Try the demo');
  });
});

@Component({
  imports: [AnalysisState],
  template: `<debrief-analysis-state [error]="error()" [empty]="empty()" emptyText="Nothing here." emptyHint="Pick a field." [suggest]="suggest()" (retry)="retried = retried + 1">content</debrief-analysis-state>`,
})
class Host {
  readonly error = signal<string | null>(null);
  readonly empty = signal(true);
  readonly suggest = signal(true);
  retried = 0;
}

describe('AnalysisState next steps', () => {
  it('shows the hint and the next steps when empty, and Retry plus next steps on error', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const fixture = TestBed.createComponent(Host);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    store.toggleLevel('Error');
    await settle(fixture);
    expect(text(fixture)).toContain('Nothing here.');
    expect(text(fixture)).toContain('Pick a field.');
    expect(text(fixture)).toContain('Clear all filters');
    fixture.componentInstance.error.set('boom');
    await settle(fixture);
    buttonByText(fixture, 'Retry').click();
    expect(fixture.componentInstance.retried).toBe(1);
    expect(text(fixture)).toContain('Clear all filters');
    fixture.componentInstance.suggest.set(false);
    await settle(fixture);
    expect(text(fixture)).not.toContain('Clear all filters');
    fixture.componentInstance.error.set(null);
    await settle(fixture);
    expect(text(fixture)).not.toContain('Clear all filters');
  });
});
