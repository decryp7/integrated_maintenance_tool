import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { ContextMenuService } from '../ui/context-menu.service';
import { TimelinePanel } from './timeline-panel';

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
    const runs = all<HTMLButtonElement>(fixture, '.lane imt-run-track .run');
    expect(runs.map((r) => r.getAttribute('data-status')).sort()).toEqual(['closed', 'closed', 'open']);
    expect(runs.every((r) => r.getAttribute('tabindex') === '-1')).toBe(true);
    const open = must(runs.find((r) => r.getAttribute('data-status') === 'open'));
    expect(el(fixture, '.lane imt-run-track .still').textContent).toContain('still running');
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
