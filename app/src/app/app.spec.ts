import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text } from '../testing/helpers';
import { App } from './app';
import { WorkspaceStore } from './core/workspace.store';
import { TimelineView } from './workspace/timeline-view';

vi.mock('echarts/core', () => ({
  use: vi.fn(),
  init: vi.fn(() => ({ setOption: vi.fn(), dispatchAction: vi.fn(), on: vi.fn(), resize: vi.fn(), dispose: vi.fn() })),
}));

describe('App', () => {
  beforeEach(() => localStorage.clear());

  async function render() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(App);
    await settle(fixture);
    return { fixture, store: TestBed.inject(WorkspaceStore) };
  }

  it('lays out the workbench and switches tabs', async () => {
    const { fixture, store } = await render();
    for (const tag of ['imt-title-bar', 'imt-activity-bar', 'imt-sidebar', 'imt-editor-tabs', 'imt-timeline-view', 'imt-inspector', 'imt-status-bar']) {
      expect(fixture.nativeElement.querySelector(tag), tag).not.toBeNull();
    }
    store.setTab('metrics');
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-metrics-view')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-inspector')).toBeNull();
    store.setTab('profile');
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-profile-editor')).not.toBeNull();
    store.toggleSidebar();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-sidebar')).toBeNull();
  });

  it('shows errors as a dismissible toast and the drop overlay', async () => {
    const { fixture, store } = await render();
    store.setError('Something failed');
    store.setDropHover(true);
    await settle(fixture);
    expect(el(fixture, '.toast').textContent).toContain('Something failed');
    expect(text(fixture)).toContain('Drop files or folders');
    el<HTMLButtonElement>(fixture, '.toast button').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.toast')).toBeNull();
  });
});

describe('TimelineView', () => {
  it('resizes the timeline with the splitter (pointer and keyboard)', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(TimelineView);
    const store = TestBed.inject(WorkspaceStore);
    store.setTimelineHeight(300);
    Object.defineProperty(fixture.nativeElement, 'clientHeight', { configurable: true, value: 800 });
    await settle(fixture);
    const splitter = el(fixture, '.splitter');
    splitter.dispatchEvent(new PointerEvent('pointerdown', { clientY: 100, pointerId: 1, bubbles: true }));
    splitter.dispatchEvent(new PointerEvent('pointermove', { clientY: 150, pointerId: 1 }));
    expect(store.timelineHeight()).toBe(350);
    splitter.dispatchEvent(new PointerEvent('pointermove', { clientY: 5000, pointerId: 1 }));
    expect(store.timelineHeight()).toBe(600);
    splitter.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    splitter.dispatchEvent(new PointerEvent('pointermove', { clientY: 0, pointerId: 1 }));
    expect(store.timelineHeight()).toBe(600);
    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp' }));
    expect(store.timelineHeight()).toBe(576);
    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    expect(store.timelineHeight()).toBe(600);
    store.setTimelineHeight(130);
    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp' }));
    expect(store.timelineHeight()).toBe(120);
    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
    expect(store.timelineHeight()).toBe(600);
    splitter.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home' }));
    expect(store.timelineHeight()).toBe(120);
    await settle(fixture);
    expect(splitter.getAttribute('aria-valuenow')).toBe('120');
    expect(splitter.getAttribute('aria-valuemax')).toBe('600');
    expect(all(fixture, 'imt-events-grid')).toHaveLength(1);
  });
});
