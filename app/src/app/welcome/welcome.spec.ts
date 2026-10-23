import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { RECENT_STORE, RecentStore } from './recent-store';
import { RecentFile } from '../core/api.types';
import { CommandRegistry } from '../extensions/command-registry';
import { RecentService } from '../core/recent/recent.service';
import { RecentOpener } from '../shell/menu/recent-opener';
import { shortPath, Welcome, WelcomeService } from './welcome';

const fakePlatform = {
  isTauri: () => false,
  pickPaths: async (dir: boolean) => [dir ? '/picked/dir' : '/picked/file.log'],
  onFilesDropped: async () => () => undefined,
};

class MemoryRecent implements RecentStore {
  items: RecentFile[] = [];
  list = () => this.items;
  add = (paths: string[]) => void (this.items = [...paths.map((path) => ({ path, openedAt: 1_700_000_000_000 })), ...this.items.filter((f) => !paths.includes(f.path))]);
  clear = () => void (this.items = []);
}

describe('Welcome', () => {
  const recent = new MemoryRecent();

  function setup() {
    recent.items = [
      { path: '/logs/web01/a.log', openedAt: new Date(2026, 9, 14, 9, 30).getTime() },
      { path: '/logs/web02', openedAt: 0 },
    ];
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: Platform, useValue: fakePlatform }, { provide: RECENT_STORE, useValue: recent }] });
    const fixture = TestBed.createComponent(Welcome);
    return { fixture, store: TestBed.inject(WorkspaceStore), welcome: TestBed.inject(WelcomeService) };
  }

  it('shortens paths to the last two segments', () => {
    expect(shortPath('/logs/web01/orders.clef')).toBe('web01/orders.clef');
    expect(shortPath('C:\\logs\\x')).toBe('logs/x');
    expect(shortPath('/')).toBe('/');
  });

  it('shows the welcome only for an empty workspace or when asked', async () => {
    const { store, welcome } = setup();
    expect(welcome.visible()).toBe(false); // not loaded yet
    await settle();
    expect(welcome.visible()).toBe(false);
    welcome.show();
    expect(welcome.visible()).toBe(true);
    welcome.hide();
    await store.clearWorkspace();
    expect(welcome.visible()).toBe(true);
  });

  // Regression: with no files open, Jobs, Notifications and AI set-up opened behind the welcome screen.
  it('steps aside for a view opened over the empty workspace, and comes back when the workspace changes', async () => {
    const { store, welcome } = setup();
    await settle();
    await store.clearWorkspace();
    expect(welcome.visible()).toBe(true);
    welcome.stepAside();
    expect(welcome.visible()).toBe(false);
    welcome.show(); // asked for again
    expect(welcome.visible()).toBe(true);
    welcome.stepAside();
    await store.openPaths(['/logs/web01/a.log']);
    await store.clearWorkspace(); // the workspace emptied again: the welcome is back
    expect(welcome.visible()).toBe(true);
  });

  it('offers open buttons, how-it-works and recent paths that reopen', async () => {
    const { fixture, store, welcome } = setup();
    await settle(fixture);
    expect(text(fixture)).toContain('How Debrief works');
    expect(all(fixture, '.steps li')).toHaveLength(4);
    expect(all(fixture, '.recent .link .mono').map((b) => b.textContent)).toEqual(['web01/a.log', 'logs/web02']);
    // J1: each recent item says when it was opened (nothing when the time is unknown).
    const whens = all(fixture, '[data-testid=recent-when]').map((b) => b.textContent?.trim());
    expect(whens[0]).toMatch(/2026/);
    expect(whens[1]).toBe('');
    expect(fixture.nativeElement.querySelector('.welcome > button')).not.toBeNull(); // back button: workspace has sources
    buttonByText(fixture, 'Open folder…').click();
    await settle(fixture);
    expect(store.summary()?.sources.some((s) => s.path === '/picked/dir')).toBe(true);
    expect(recent.items[0]?.path).toBe('/picked/dir');
    buttonByText(fixture, 'Open files…').click();
    await settle(fixture);
    expect(recent.items[0]?.path).toBe('/picked/file.log');
    welcome.show();
    el<HTMLButtonElement>(fixture, '.recent .link').click();
    await settle(fixture);
    expect(welcome.visible()).toBe(false);
    expect(store.summary()?.sources.some((s) => s.path === '/picked/file.log')).toBe(true);
    buttonByText(fixture, 'Back to workspace').click();
  });

  it('opens a project from its button, and lists recent projects before files (J1)', async () => {
    const { fixture, welcome } = setup();
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(undefined as never);
    await settle(fixture);
    TestBed.inject(RecentService).projects.set([{ path: '/p/incident/case.debriefproj', name: 'Case 12', openedAt: Date.now() }]);
    await settle(fixture);
    buttonByText(fixture, 'Open project…').click();
    expect(run).toHaveBeenCalledWith('project.open');
    const links = all(fixture, '.recent .link').map((b) => b.textContent?.replace(/\s+/g, ' ').trim());
    expect(links).toHaveLength(3);
    expect(links[0]).toContain('Case 12');
    expect(links[0]).toContain('incident/case.debriefproj');
    expect(links[1]).toContain('web01/a.log');
    expect(links[2]).toContain('logs/web02');
    const openProject = vi.spyOn(TestBed.inject(RecentOpener), 'openProject').mockResolvedValue();
    welcome.show();
    el<HTMLButtonElement>(fixture, '[data-testid=welcome-recent-project]').click();
    expect(openProject).toHaveBeenCalledWith('/p/incident/case.debriefproj');
    expect(welcome.visible()).toBe(false);
  });

  it('has no back button or recent list when empty', async () => {
    const { fixture, store } = setup();
    recent.items = [];
    await settle(fixture);
    await store.clearWorkspace();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.recent')).toBeNull();
    expect(all(fixture, 'button').map((b) => b.textContent?.trim())).toEqual(['Open folder…', 'Open files…', 'Open project…', 'Try a demo', '?']);
  });

  it('does not remember paths that failed to open', async () => {
    const { fixture, store } = setup();
    await settle(fixture);
    recent.items = [];
    vi.spyOn(TestBed.inject(Platform), 'pickPaths').mockResolvedValue(['/bad']);
    const { DebriefBackend } = await import('../core/backend');
    vi.spyOn(TestBed.inject(DebriefBackend), 'openPaths').mockRejectedValue('nope');
    await store.pickAndOpen(true);
    await store.openPaths(['/bad2']);
    expect(recent.items).toEqual([]);
    expect(store.error()).toBe('nope');
  });
});
