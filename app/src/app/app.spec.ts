import { TestBed } from '@angular/core/testing';
import { el, provideDockTesting, settle, text } from '../testing/helpers';
import { App } from './app';
import { WorkspaceStore } from './core/workspace.store';
import { DockService } from './dock/dock.service';
import { provideToolsBackend } from './core/tools-backend';
import { LifecycleService } from './lifecycle/lifecycle.service';

describe('App', () => {
  beforeEach(() => localStorage.clear());

  async function render() {
    TestBed.configureTestingModule({ providers: [provideDockTesting(), provideToolsBackend()] });
    const fixture = TestBed.createComponent(App);
    await settle(fixture, 8);
    await vi.waitFor(() => {
      for (const tag of ['imt-timeline-panel', 'imt-events-grid', 'imt-inspector']) expect(fixture.nativeElement.querySelector(tag), tag).not.toBeNull();
    });
    return { fixture, store: TestBed.inject(WorkspaceStore), dock: TestBed.inject(DockService) };
  }

  it('lays out the workbench with the default dock panels', async () => {
    const { fixture, dock } = await render();
    for (const tag of ['imt-title-bar', 'imt-activity-bar', 'imt-sidebar', 'imt-filter-bar', 'imt-dock-host', 'imt-status-bar', 'imt-command-palette']) {
      expect(fixture.nativeElement.querySelector(tag), tag).not.toBeNull();
    }
    expect([...dock.openIds()].sort()).toEqual(['events', 'inspector', 'metrics', 'profile', 'timeline']);
    expect(fixture.nativeElement.querySelector('imt-timeline-panel')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-inspector')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-events-grid')).not.toBeNull();
  });

  it('shows the welcome over the dock when the workspace is empty', async () => {
    const { fixture, store } = await render();
    expect(fixture.nativeElement.querySelector('imt-welcome')).toBeNull();
    await store.clearWorkspace();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-welcome')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-dock-host')).not.toBeNull();
  });

  it('toggles the sidebar', async () => {
    const { fixture, store } = await render();
    store.toggleSidebar();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-sidebar')).toBeNull();
  });

  it('shows errors, notices as dismissible toasts and the drop overlay', async () => {
    const { fixture, store } = await render();
    store.setError('Something failed');
    store.setNotice('Exported 3 events');
    store.setDropHover(true);
    await settle(fixture);
    expect(el(fixture, '.toast[role=alert]').textContent).toContain('Something failed');
    expect(el(fixture, '.toast[role=status]').textContent).toContain('Exported 3 events');
    expect(text(fixture)).toContain('Drop files or folders');
    el<HTMLButtonElement>(fixture, '.toast[role=alert] button').click();
    el<HTMLButtonElement>(fixture, '.toast[role=status] button').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.toast')).toBeNull();
  });

  it('renders the lifecycle banners right below the title bar, as part of the shell', async () => {
    const { fixture } = await render();
    const host = fixture.nativeElement as HTMLElement;
    const banners = host.querySelector('imt-lifecycle-banners');
    expect(banners?.previousElementSibling?.tagName.toLowerCase()).toBe('imt-title-bar');
    expect(host.querySelector('imt-title-bar imt-title-extras')).not.toBeNull();
    TestBed.inject(LifecycleService).status.set({ version: '0.1.0', safeMode: true, previousCrash: false, sessionAvailable: false });
    await settle(fixture);
    expect(banners?.querySelector('[data-testid=safe-mode-banner]')).not.toBeNull();
  });
});
