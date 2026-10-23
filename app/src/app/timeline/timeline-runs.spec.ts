import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { ContextMenuService } from '../ui/context-menu.service';
import { TimelinePanel } from './timeline-panel';
import { anchors, NoteActions } from '../notes/note-actions';
import { SelectionStore } from '../core/workspace/selection.store';

async function render(backend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
  const fixture = TestBed.createComponent(TimelinePanel);
  const store = TestBed.inject(WorkspaceStore);
  await settle(fixture, 8);
  return { fixture, store };
}

describe('TimelinePanel runs', () => {
  it('draws runs as bands per lane behind the markers, out of the tab order', async () => {
    const { fixture } = await render();
    const runs = all<HTMLButtonElement>(fixture, '.lane debrief-run-track .run');
    expect(runs.map((r) => r.getAttribute('data-status')).sort()).toEqual(['closed', 'closed', 'open']);
    expect(runs.every((r) => r.getAttribute('tabindex') === '-1')).toBe(true);
    const open = must(runs.find((r) => r.getAttribute('data-status') === 'open'));
    // "still running" at the band's end needs room (RUN_STILL_MIN_PX); the name always carries it.
    expect(open.getAttribute('aria-label')).toBe('Running (Still running) — show this run');
    // Runs are not listed among the plugin annotations.
    expect(all(fixture, '.ann-legend [data-kind="run"]')).toHaveLength(0);
  });

  it('shows a tooltip with times, duration and messages, and filters to the run on click', async () => {
    const { fixture, store } = await render();
    const closed = must(all<HTMLButtonElement>(fixture, '.lane .run').find((r) => r.getAttribute('data-status') === 'closed'));
    closed.dispatchEvent(new PointerEvent('pointerenter', { clientX: 10, clientY: 10 }));
    await settle(fixture);
    const tip = el(fixture, '[data-testid="run-tip"]');
    expect(tip.textContent).toContain('Running');
    expect(tip.textContent).toContain('Stopped');
    expect(tip.textContent).toContain('Started');
    expect(tip.textContent).toContain('Application starting');
    closed.dispatchEvent(new PointerEvent('pointerleave'));
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid="run-tip"]')).toBeNull();
    closed.click();
    await settle(fixture);
    expect(store.query().groups).toHaveLength(1);
    expect(store.anchor()).not.toBeNull();
  });

  it('hides runs with the legend toggles and lists them in the Evidence menu', async () => {
    const { fixture, store } = await render();
    const toggle = el<HTMLButtonElement>(fixture, '[data-testid="runs-toggle"]');
    expect(toggle.getAttribute('aria-pressed')).toBe('true');
    // The one-line strip keeps the run kinds in the legend popover.
    el<HTMLButtonElement>(fixture, '[data-testid=legend-more]').click();
    await settle(fixture);
    const kind = el<HTMLButtonElement>(fixture, '.run-legend [data-run-kind="run"]');
    expect(kind.textContent).toContain('Running');
    kind.click();
    await settle(fixture);
    expect(all(fixture, '.lane .run')).toHaveLength(0);
    kind.click();
    await settle(fixture);
    expect(all(fixture, '.lane .run')).toHaveLength(3);
    toggle.click();
    await settle(fixture);
    expect(all(fixture, '.lane .run')).toHaveLength(0);
    expect(fixture.nativeElement.querySelector('.run-legend [data-run-kind]')).toBeNull();
    toggle.click();
    await settle(fixture);

    const menu = TestBed.inject(ContextMenuService);
    const open = vi.spyOn(menu, 'open');
    el<HTMLButtonElement>(fixture, '.ann-legend .ann-list').click();
    const items = at(open.mock.calls, 0)[2];
    const run = must(items.find((i) => i.label.startsWith('Running (still running)')));
    run.run();
    await settle(fixture);
    expect(store.query().groups).toEqual(['WEB01 · 200']);
  });

  it('stacks overlapping runs of one lane on labelled sub-tracks and makes the lane taller by that many rows', async () => {
    const backend = tinyBackend();
    const original = backend.annotations.bind(backend);
    vi.spyOn(backend, 'annotations').mockImplementation(async (r) => {
      const all = await original(r);
      const first = must(all.find((a) => a.run && a.run.status === 'closed'));
      // Two views open at once on the same lane, over the first run.
      const view = (value: string, from: number, to: number) => ({ ...first, kind: 'view', from, to, run: { ...first.run!, span: 'view', spanLabel: 'View', group: { field: 'View', value } } });
      const span = first.to - first.from;
      return [...all, view('Settings', first.from, first.to), view('Help', first.from + span / 4, first.to)];
    });
    const { fixture } = await render(backend);
    const lane = must(all<HTMLElement>(fixture, '.lane').find((l) => l.querySelector('.run[data-kind="view"]')));
    const rows = all<HTMLElement>(fixture, '.lane .run').filter((r) => lane.contains(r));
    // The run and the two views overlap: three sub-tracks, one per run.
    expect(new Set(rows.map((r) => r.getAttribute('data-row')))).toEqual(new Set(['0', '1', '2']));
    expect(lane.style.getPropertyValue('--run-rows')).toBe('3');
    const labels = all(fixture, '.run-label').filter((l) => lane.contains(l)).map((l) => l.textContent?.trim());
    expect(labels).toEqual(expect.arrayContaining(['Settings', 'Help']));
    // The visible label is in the accessible name (WCAG 2.5.3).
    expect(must(rows.find((r) => r.textContent?.includes('Help'))).getAttribute('aria-label')).toMatch(/^Help \(/);
  });

  it('offers the Evidence menu in the runs legend when there are no plugin annotations', async () => {
    const backend = tinyBackend();
    const original = backend.annotations.bind(backend);
    vi.spyOn(backend, 'annotations').mockImplementation(async (r) => (await original(r)).filter((a) => a.run));
    const { fixture } = await render(backend);
    expect(fixture.nativeElement.querySelector('.ann-legend')).toBeNull();
    expect(el(fixture, '.run-legend .ann-list')).toBeTruthy();
    expect(el(fixture, '.run-legend .lg[title]').title).toContain('Hide the runs');
  });
});

describe('TimelinePanel notes on markers and spans', () => {
  const menuAt = (target: Element) => {
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 40 });
    target.dispatchEvent(event);
    return event;
  };

  it('offers "Add note…" on a marker, a run and an annotation, anchored to its time', async () => {
    const { fixture } = await render();
    const menu = TestBed.inject(ContextMenuService);
    const write = vi.spyOn(TestBed.inject(NoteActions), 'write').mockResolvedValue(null);
    const jump = vi.spyOn(TestBed.inject(SelectionStore), 'jumpTo');

    // A marker: the note names its kind and time; "Show its event" jumps to it.
    const marker = must(all<HTMLButtonElement>(fixture, '.marker')[0]);
    expect(menuAt(marker).defaultPrevented).toBe(true);
    const onMarker = menu.menu()!.items;
    expect(onMarker.map((i) => i.label)).toEqual(['Add note to this marker…', 'Show its event']);
    at(onMarker, 0).run();
    expect(write).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'marker', to: null }));
    at(onMarker, 1).run();
    expect(jump).toHaveBeenCalled();

    // A closed run: the note covers its span.
    const run = must(all<HTMLButtonElement>(fixture, '.lane .run').find((r) => r.getAttribute('data-status') === 'closed'));
    expect(menuAt(run).defaultPrevented).toBe(true);
    const onRun = menu.menu()!.items;
    expect(onRun.map((i) => i.label)).toEqual(['Add note to this span…', 'Show this run']);
    at(onRun, 0).run();
    const anchor = write.mock.lastCall![0];
    expect(anchor.kind === 'marker' && anchor.to !== null && anchor.to > anchor.from).toBe(true);
    at(onRun, 1).run();

    // A plugin annotation: the note covers it; the other entry shows its evidence.
    const ann = must(all<HTMLButtonElement>(fixture, '.ann')[0]);
    menuAt(ann);
    const onAnn = menu.menu()!.items;
    expect(onAnn.map((i) => i.label)).toEqual(['Add note to this span…', 'Show the evidence']);
    at(onAnn, 0).run();
    expect(write).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'marker' }));
    at(onAnn, 1).run();
    await settle(fixture);
  });

  it('anchors a marker note on its time and a span note on its range', () => {
    expect(anchors.marker('Crash', 10.4)).toEqual({ kind: 'marker', name: 'Crash', from: 10, to: null });
    expect(anchors.marker('Run', 10, 20.6)).toEqual({ kind: 'marker', name: 'Run', from: 10, to: 21 });
    expect(anchors.marker('Point', 10, 10)).toEqual({ kind: 'marker', name: 'Point', from: 10, to: null });
    expect(anchors.chartPoint('CPU', null, 5.5, null)).toEqual({ kind: 'chartPoint', chart: 'CPU', series: null, at: 6, value: null });
  });
});
