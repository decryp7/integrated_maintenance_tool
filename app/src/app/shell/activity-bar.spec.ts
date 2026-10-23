import { signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, el, FakeDock, provideFakeDock, provideTinyBackend, settle, testLogger } from '../../testing/helpers';
import { aiActivityItems } from '../ai/ai-activity';
import { analysisActivityItems } from '../analysis/analysis-activity';
import { ANALYSIS_MENU_ORDER } from '../analysis/analysis-menu-order';
import { ANALYSIS_VIEWS } from '../analysis/analysis.plugin';
import { TOOL_IDS } from '../core/tools.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_ACTIVITY_ITEMS, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefActivityItem } from '../extensions/tokens';
import { liveActivityItems } from '../live/live-activity';
import { SCHEDULES_VIEWS } from '../schedules/provide-schedules';
import { toolsActivityItems } from '../tools/tools-activity';
import { toolsViews } from '../tools/tools-commands';
import { DialogService } from '../ui/dialog.service';
import { LiveTail } from '../workspace/live-tail';
import { ActivityBar } from './activity-bar';
import { fittingTopItems } from './activity-overflow';

const KEY_CODES: Record<string, number> = { ArrowDown: 40, ArrowUp: 38, ArrowRight: 39, Escape: 27, Enter: 13 };

function press(target: Element, key: string): void {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, keyCode: KEY_CODES[key], bubbles: true } as KeyboardEventInit));
}

interface Setup {
  plugin?: DebriefActivityItem[];
}

function setup(options: Setup = {}) {
  const dock = new FakeDock();
  const live = { enabled: signal(false), available: signal(false) };
  const follow = { active: signal(false) };
  const ai = { hasModel: signal(false) };
  const ran: string[] = [];
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(),
      provideFakeDock(dock),
      { provide: DEBRIEF_VIEWS, multi: true, useValue: ANALYSIS_VIEWS },
      { provide: DEBRIEF_VIEWS, multi: true, useValue: toolsViews({ listed: (id) => id !== TOOL_IDS.eventLog }) },
      { provide: DEBRIEF_VIEWS, multi: true, useValue: SCHEDULES_VIEWS },
      { provide: DEBRIEF_VIEWS, multi: true, useValue: [{ id: 'ai-chat', category: 'AI', title: 'AI assistant', icon: 'ai', component: async () => ActivityBar }] },
      {
        provide: DEBRIEF_COMMANDS,
        multi: true,
        useValue: [
          { id: 'live.toggle', title: 'Toggle live tail', when: () => live.available(), run: () => live.enabled.set(!live.enabled()) },
          { id: 'tools.collect', title: 'Collect diagnostics bundle…', run: () => void ran.push('tools.collect') },
        ],
      },
      { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: analysisActivityItems },
      { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: toolsActivityItems },
      { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: () => liveActivityItems(live as unknown as LiveTail, follow as never) },
      { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: () => aiActivityItems(ai as never, 'ai-chat') },
      { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useValue: options.plugin ?? [] },
    ],
  });
  const fixture = TestBed.createComponent(ActivityBar);
  document.body.appendChild(fixture.nativeElement);
  return { fixture, dock, live, follow, ai, ran };
}

const labels = (fixture: ComponentFixture<ActivityBar>) => all(fixture, 'button.act').map((b) => b.getAttribute('aria-label'));
const button = (fixture: ComponentFixture<ActivityBar>, label: string) => el<HTMLButtonElement>(fixture, `button.act[aria-label="${label}"]`);
const menu = (label: string) => document.querySelector<HTMLElement>(`[role=menu][aria-label="${label}"]`);
const entries = (label: string) => [...(menu(label)?.querySelectorAll<HTMLElement>('[role^=menuitem]') ?? [])];
const entry = (label: string, name: string) => entries(label).find((e) => e.querySelector('.label')?.textContent?.trim() === name);
const entryLabels = (label: string) => entries(label).map((e) => e.querySelector('.label')?.textContent?.trim());

describe('ActivityBar', () => {
  afterEach(() => {
    document.querySelectorAll('debrief-activity-bar, .cdk-overlay-container').forEach((n) => n.remove());
  });

  it('renders the contributed buttons in order, Settings at the bottom, AI hidden until a model is set up', async () => {
    const { fixture, ai, live } = setup();
    await settle(fixture);
    // Live tail shows once there are files to follow.
    expect(labels(fixture)).toEqual(['Explorer', 'Search', 'Timeline', 'Analysis', 'Metrics', 'Tools', 'Profile editor', 'Settings']);
    live.available.set(true);
    await settle(fixture);
    expect(labels(fixture)).toEqual(['Explorer', 'Search', 'Timeline', 'Analysis', 'Metrics', 'Toggle live tail', 'Tools', 'Profile editor', 'Settings']);
    expect(all<HTMLButtonElement>(fixture, 'button.act').every((b) => b.type === 'button')).toBe(true);
    ai.hasModel.set(true);
    await settle(fixture);
    expect(labels(fixture)).toContain('AI assistant');
    expect(labels(fixture).indexOf('AI assistant')).toBe(labels(fixture).indexOf('Toggle live tail') + 1);
  });

  it('switches views, focuses search, toggles the explorer and opens settings', async () => {
    const { fixture, dock } = setup();
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    button(fixture, 'Metrics').click();
    button(fixture, 'Profile editor').click();
    button(fixture, 'Timeline').click();
    expect(dock.calls).toEqual(['show:metrics', 'show:profile', 'show:timeline']);
    button(fixture, 'Search').click();
    button(fixture, 'Explorer').click();
    button(fixture, 'Settings').click();
    await settle(fixture);
    expect(store.focusSearchNonce()).toBe(1);
    expect(store.sidebarOpen()).toBe(false);
    expect(button(fixture, 'Explorer').getAttribute('aria-pressed')).toBe('false');
    // The Settings screen is a lazy chunk: the window opens once it has loaded.
    await vi.waitFor(() => expect(TestBed.inject(DialogService).stack().map((d) => d.options.title)).toEqual(['Settings']));
  });

  it('marks the open views and puts the shortcut in the tooltip', async () => {
    const { fixture, dock } = setup();
    await settle(fixture);
    expect(button(fixture, 'Timeline').classList).toContain('on');
    expect(button(fixture, 'Metrics').classList).not.toContain('on');
    dock.openIds.set(new Set(['metrics']));
    await settle(fixture);
    expect(button(fixture, 'Metrics').classList).toContain('on');
    expect(button(fixture, 'Explorer').title).toBe('Explorer (Ctrl+B)');
    expect(button(fixture, 'Explorer').getAttribute('aria-keyshortcuts')).toBe('Control+B');
    expect(button(fixture, 'Search').title).toBe('Search (/)');
    expect(button(fixture, 'Settings').title).toBe('Settings (Ctrl+,)');
    expect(button(fixture, 'Timeline').title).toBe('Timeline');
    expect(button(fixture, 'Timeline').hasAttribute('aria-keyshortcuts')).toBe(false);
  });

  it('marks a view that is open but not in front with a dot instead of lighting the button', async () => {
    const { fixture, dock } = setup();
    dock.openIds.set(new Set(['timeline', 'metrics']));
    await settle(fixture);
    const dot = (label: string) => button(fixture, label).querySelector('[data-testid=open-elsewhere]');
    expect(button(fixture, 'Metrics').classList).toContain('on');
    expect(dot('Metrics')).toBeNull();
    dock.hidden.add('metrics');
    dock.openIds.set(new Set(['timeline', 'metrics']));
    await settle(fixture);
    expect(button(fixture, 'Metrics').classList).not.toContain('on');
    expect(dot('Metrics')).not.toBeNull();
    expect(dot('Timeline')).toBeNull();
    // Closed altogether: neither lit nor dotted.
    dock.openIds.set(new Set(['timeline']));
    await settle(fixture);
    expect(dot('Metrics')).toBeNull();
  });

  it('dots a fly-out button while one of its views is open only behind others', async () => {
    const { fixture, dock } = setup();
    dock.openIds.set(new Set(['patterns']));
    await settle(fixture);
    expect(button(fixture, 'Analysis').classList).toContain('on');
    expect(button(fixture, 'Analysis').querySelector('[data-testid=open-elsewhere]')).toBeNull();
    dock.hidden.add('patterns');
    dock.openIds.set(new Set(['patterns']));
    await settle(fixture);
    expect(button(fixture, 'Analysis').querySelector('[data-testid=open-elsewhere]')).not.toBeNull();
  });

  it('has no Jobs or Notifications buttons: the status bar keeps them', async () => {
    const { fixture } = setup();
    await settle(fixture);
    expect(labels(fixture)).not.toContain('Jobs');
    expect(labels(fixture)).not.toContain('Notifications');
  });

  it('lists the analysis views from the registry, checks the open ones and shows the chosen one', async () => {
    const { fixture, dock } = setup();
    dock.openIds.set(new Set(['patterns']));
    await settle(fixture);
    const analysis = button(fixture, 'Analysis');
    expect(analysis.getAttribute('aria-haspopup')).toBe('menu');
    expect(analysis.classList).toContain('on');
    analysis.click();
    await settle(fixture);
    // The ordered analysis views, then the other views of the category (Parsers, and Saved queries from the tools feature).
    expect(entryLabels('Analysis')).toEqual([...ANALYSIS_MENU_ORDER.map((id) => ANALYSIS_VIEWS.find((v) => v.id === id)?.title), 'Parsers', 'Saved queries']);
    expect(entry('Analysis', 'Patterns')?.getAttribute('aria-checked')).toBe('true');
    expect(entry('Analysis', 'Anomalies')?.getAttribute('aria-checked')).toBe('false');
    expect(entry('Analysis', 'Patterns')?.querySelector('debrief-icon.mi-icon')).toBeTruthy();
    entry('Analysis', 'Anomalies')?.click();
    await settle(fixture);
    expect(dock.calls).toEqual(['show:anomalies']);
    expect(menu('Analysis')).toBeNull();
  });

  it('opens a fly-out from the keyboard, moves with the arrows and closes with Escape back to its button', async () => {
    const { fixture } = setup();
    await settle(fixture);
    const analysis = button(fixture, 'Analysis');
    analysis.focus();
    press(analysis, 'ArrowRight');
    await settle(fixture);
    expect(analysis.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement?.querySelector('.label')?.textContent?.trim()).toBe('Query results');
    press(document.activeElement!, 'ArrowDown');
    await settle(fixture);
    expect(document.activeElement?.querySelector('.label')?.textContent?.trim()).toBe('Moment');
    press(document.activeElement!, 'Escape');
    await settle(fixture);
    expect(menu('Analysis')).toBeNull();
    expect(document.activeElement).toBe(analysis);
  });

  it('lists only the tools this platform offers, then the collector, and opens the chosen tool', async () => {
    const { fixture, dock, ran } = setup();
    await settle(fixture);
    button(fixture, 'Tools').click();
    await settle(fixture);
    expect(entryLabels('Tools')).toEqual(['Process Monitor', 'Performance Monitor', 'Collection schedules', 'Collection runs', 'Collect diagnostics bundle…']);
    expect(entry('Tools', 'Collect diagnostics bundle…')?.getAttribute('role')).toBe('menuitem');
    entry('Tools', 'Process Monitor')?.click();
    await settle(fixture);
    expect(dock.calls).toEqual(['show:process-monitor']);
    button(fixture, 'Tools').click();
    await settle(fixture);
    entry('Tools', 'Collect diagnostics bundle…')?.click();
    await settle(fixture);
    expect(ran).toEqual(['tools.collect']);
  });

  it('toggles live tail: hidden while nothing is open, pressed while on, pulsing while following', async () => {
    const { fixture, live, follow } = setup();
    await settle(fixture);
    const toggle = () => button(fixture, 'Toggle live tail');
    expect(fixture.nativeElement.querySelector('[data-activity-id="live-tail"]')).toBeNull();

    live.available.set(true);
    await settle(fixture);
    expect(toggle().hasAttribute('aria-disabled')).toBe(false);
    expect(toggle().getAttribute('aria-pressed')).toBe('false');
    toggle().click();
    await settle(fixture);
    expect(live.enabled()).toBe(true);
    expect(toggle().getAttribute('aria-pressed')).toBe('true');
    expect(toggle().classList).toContain('on');
    expect(toggle().querySelector('.pulse')).toBeNull();
    follow.active.set(true);
    await settle(fixture);
    expect(toggle().querySelector('.pulse')).toBeTruthy();
  });

  it('opens the AI chat once a model is set up', async () => {
    const { fixture, ai, dock } = setup();
    ai.hasModel.set(true);
    await settle(fixture);
    button(fixture, 'AI assistant').click();
    expect(dock.calls).toEqual(['show:ai-chat']);
  });

  it('shows plugin items with their badge and keeps rendering when a plugin item throws', async () => {
    const count = signal(3);
    const { fixture } = setup({
      plugin: [
        { id: 'example', label: 'Example plugin', icon: 'puzzle', order: 450, badge: () => count(), run: { view: 'metrics' } },
        {
          id: 'broken',
          label: 'Broken plugin',
          icon: 'puzzle',
          order: 460,
          when: () => {
            throw new Error('bug');
          },
          run: { command: 'x' },
        },
        {
          id: 'broken-badge',
          label: 'Broken badge',
          icon: 'puzzle',
          order: 470,
          badge: () => {
            throw new Error('bug');
          },
          run: { view: 'metrics' },
        },
      ],
    });
    await settle(fixture);
    const names = labels(fixture);
    expect(names.indexOf('Example plugin, 3')).toBe(names.indexOf('Analysis') + 1);
    expect(names).not.toContain('Broken plugin');
    expect(names).toContain('Broken badge');
    expect(names).toContain('Settings');
    expect(button(fixture, 'Example plugin, 3').querySelector('.badge')?.textContent).toBe('3');
    expect(button(fixture, 'Broken badge').querySelector('.badge')).toBeNull();
    count.set(0);
    await settle(fixture);
    expect(button(fixture, 'Example plugin, 0').querySelector('.badge')?.textContent).toBe('0');
    expect(testLogger().entries.filter((e) => e.message === 'Activity item failed').length).toBe(2);
  });

  it('collapses the last top buttons into a More fly-out when the bar is too short', async () => {
    const { fixture, dock, live } = setup();
    live.available.set(true);
    await settle(fixture);
    // 8 top buttons + Settings; 6 slots fit in 264px, so 4 stay and More holds the rest.
    (fixture.componentInstance as unknown as { barHeight: { set(v: number): void } }).barHeight.set(264);
    await settle(fixture);
    expect(labels(fixture)).toEqual(['Explorer', 'Search', 'Timeline', 'Analysis', 'More', 'Settings']);
    const more = button(fixture, 'More');
    more.click();
    await settle(fixture);
    expect(entryLabels('More')).toEqual(['Metrics', 'Toggle live tail', 'Tools', 'Profile editor']);
    expect(entry('More', 'Toggle live tail')?.getAttribute('role')).toBe('menuitemcheckbox');
    expect(entry('More', 'Tools')?.getAttribute('aria-haspopup')).toBe('menu');
    entry('More', 'Profile editor')?.click();
    await settle(fixture);
    expect(dock.calls).toEqual(['show:profile']);
  });
});

describe('fittingTopItems', () => {
  it('shows every button when they fit or the height is unknown, else leaves a slot for More', () => {
    expect(fittingTopItems(8, 1, 0)).toBe(8);
    expect(fittingTopItems(8, 1, 1000)).toBe(8);
    // 9 slots of 42px (minus one gap) plus 16px padding = 388px: exactly enough.
    expect(fittingTopItems(8, 1, 388)).toBe(8);
    expect(fittingTopItems(8, 1, 387)).toBe(6);
    expect(fittingTopItems(8, 1, 264)).toBe(4);
    expect(fittingTopItems(8, 1, 40)).toBe(0);
  });
});
