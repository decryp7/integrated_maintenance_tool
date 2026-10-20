import { TestBed } from '@angular/core/testing';
import { all, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { Finding } from '../core/api.types';
import { LAYOUT_MODE_KEY, LayoutModeStore } from '../core/workspace/layout-mode';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { NotificationsStore } from '../notifications/notifications.store';
import { headlineOf, OverviewHeadline } from '../overview/overview-headline';
import { OverviewActions } from '../overview/overview-actions';
import { ShareBackend } from '../share/share-backend';
import { TimelinePanel } from '../timeline/timeline-panel';
import { LEGEND_COLLAPSED } from '../timeline/timeline-legend';
import { at } from '../../testing/must';

const finding = (kind: string, title = kind): Finding => ({ kind, severity: 'critical', title, detail: '', action: { kind: 'zoom', from: 1, to: 2, recordId: 3 } });

describe('focused first run', () => {
  afterEach(() => localStorage.removeItem(LAYOUT_MODE_KEY));

  it('View › Layout switches between Focused (default) and Full, and remembers it', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const layout = TestBed.inject(LayoutModeStore);
    expect(layout.mode()).toBe('focused');
    await TestBed.inject(CommandRegistry).run('view.layout.full');
    expect(layout.full()).toBe(true);
    expect(localStorage.getItem(LAYOUT_MODE_KEY)).toBe('full');
    await TestBed.inject(CommandRegistry).run('view.layout.focused');
    expect(layout.mode()).toBe('focused');
  });

  it('keeps local messages in the bell: unread until read, all read, dismissed', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const notices = TestBed.inject(NotificationsStore);
    const before = notices.unread();
    const ran: string[] = [];
    notices.postLocal({ key: 'k', source: 'ai', severity: 'info', title: 'T', actionLabel: 'Do', run: () => ran.push('do') });
    notices.postLocal({ key: 'k', source: 'ai', severity: 'info', title: 'T2', actionLabel: 'Do', run: () => ran.push('do') });
    expect(notices.notices().filter((n) => n.dedupeKey === 'local:k').map((n) => n.title)).toEqual(['T2']);
    expect(notices.unread()).toBe(before + 1);
    const id = at(notices.notices(), 0).id;
    expect(id).toBeLessThan(0);
    void notices.markRead(id);
    expect(notices.unread()).toBe(before);
    notices.postLocal({ key: 'j', source: 'ai', severity: 'info', title: 'J', actionLabel: 'Go', run: () => ran.push('go') });
    void notices.markAllRead();
    expect(notices.unread()).toBe(before);
    notices.withdrawLocal('j');
    expect(notices.notices().some((n) => n.title === 'J')).toBe(false);
    notices.runLocal(id);
    expect(ran).toEqual(['do']);
    expect(notices.localAction(id)).toBeNull();
  });
});

describe('timeline legend', () => {
  it('lists only the marker kinds in the data, on one line, with an accessible "+N more" toggle', async () => {
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    let kinds: string[] = [];
    vi.spyOn(backend, 'queryTimeline').mockImplementation(async (req) => {
      const t = await real(req);
      at(t.lanes, 0).markers = kinds.map((kind, i) => ({ kind, bucket: i, ts: t.from + i, id: i, count: 1, message: '' }));
      for (const lane of t.lanes.slice(1)) lane.markers = [];
      return t;
    });
    const realSummary = backend.workspaceSummary.bind(backend);
    vi.spyOn(backend, 'workspaceSummary').mockImplementation(async () => {
      const s = await realSummary();
      const base = at(s.markerKinds, 0);
      return { ...s, markerKinds: [...s.markerKinds, ...['x1', 'x2', 'x3'].map((kind) => ({ ...base, kind, label: kind.toUpperCase() }))] };
    });
    const summary = await backend.workspaceSummary();
    const all8 = summary.markerKinds.map((k) => k.kind);
    expect(all8.length).toBeGreaterThan(LEGEND_COLLAPSED);
    kinds = all8.slice(0, LEGEND_COLLAPSED + 2);
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    const legendButtons = () => all<HTMLButtonElement>(fixture, '.legend .lg:not(.more)');
    expect(legendButtons()).toHaveLength(LEGEND_COLLAPSED);
    const more = el<HTMLButtonElement>(fixture, '[data-testid=legend-more]');
    expect(more.textContent?.trim()).toBe('+2 more');
    expect(more.getAttribute('aria-expanded')).toBe('false');
    expect(el(fixture, '#' + more.getAttribute('aria-controls')).classList.contains('collapsed')).toBe(true);
    more.click();
    await settle(fixture);
    expect(legendButtons()).toHaveLength(LEGEND_COLLAPSED + 2);
    expect(more.getAttribute('aria-expanded')).toBe('true');
    expect(more.textContent?.trim()).toBe('Show fewer');
    fixture.destroy();
  });
});

describe('OverviewHeadline', () => {
  it('summarises crashes, errors and silences, and is hidden when nothing is notable', () => {
    expect(headlineOf([], { Error: 0, Fatal: 0 })).toBeNull();
    expect(headlineOf([finding('crash'), finding('silence'), finding('memoryGrowth')], { Error: 3, Fatal: 1 })).toMatchObject({ errors: 4 });
    expect(headlineOf([finding('silence')], null)?.silent).toHaveLength(1);
  });

  async function mount(findings: Finding[]) {
    const dock = new FakeDock();
    const views: string[] = [];
    const queries: unknown[] = [];
    const share = { overview: vi.fn(async () => findings) };
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        provideFakeDock(dock),
        { provide: ShareBackend, useValue: share },
        { provide: COMMAND_CONTEXT, useValue: { showView: (id: string) => views.push(id), setQuery: (q: unknown) => queries.push(q), notifyError: () => undefined, summary: null } },
      ],
    });
    const actions = { run: vi.fn(async () => undefined) };
    TestBed.overrideProvider(OverviewActions, { useValue: actions });
    TestBed.inject(WorkspaceStore);
    const fixture = TestBed.createComponent(OverviewHeadline);
    await settle(fixture);
    return { fixture, share, views, queries, actions };
  }

  it('shows one toolbar line whose counts act like the Overview, and Review opens it', async () => {
    const { fixture, share, views, queries, actions } = await mount([finding('crash', 'a'), finding('crash', 'b'), finding('silence', 's')]);
    expect(share.overview).toHaveBeenCalledTimes(1);
    const bar = el(fixture, '[role=toolbar]');
    expect(bar.getAttribute('aria-label')).toBe('Health check');
    expect(text(fixture)).toMatch(/2 crashes.*errors.*1 went silent.*Review/);
    el<HTMLButtonElement>(fixture, '[data-testid=headline-crashes]').click();
    el<HTMLButtonElement>(fixture, '[data-testid=headline-crashes]').click();
    el<HTMLButtonElement>(fixture, '[data-testid=headline-crashes]').click();
    expect(actions.run.mock.calls.map((c) => { const [f, id] = c as unknown as [Finding, string]; return `${f.title}:${id}`; })).toEqual(['a:jump', 'b:jump', 'a:jump']);
    el<HTMLButtonElement>(fixture, '[data-testid=headline-silent]').click();
    expect((actions.run.mock.calls.at(-1) as unknown[])[1]).toBe('showRange');
    el<HTMLButtonElement>(fixture, '[data-testid=headline-errors]').click();
    expect(queries).toEqual([{ levels: ['Error', 'Fatal'] }]);
    el<HTMLButtonElement>(fixture, '[data-testid=headline-review]').click();
    expect(views).toEqual(['events', 'overview']);
  });

  it('uses singular forms for one', async () => {
    const { fixture } = await mount([finding('crash')]);
    expect(el(fixture, '[data-testid=headline-crashes]').textContent?.trim()).toBe('1 crash');
  });
});
