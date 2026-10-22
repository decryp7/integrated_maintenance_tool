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
    // The panels load lazily: a busy machine needs more than the default second.
    await vi.waitFor(
      () => {
        for (const tag of ['imt-timeline-panel', 'imt-events-grid']) expect(fixture.nativeElement.querySelector(tag), tag).not.toBeNull();
      },
      { timeout: 10_000 },
    );
    return { fixture, store: TestBed.inject(WorkspaceStore), dock: TestBed.inject(DockService) };
  }

  it('lays out the workbench with the default dock panels', async () => {
    const { fixture, dock } = await render();
    for (const tag of ['imt-title-bar', 'imt-activity-bar', 'imt-sidebar', 'imt-filter-bar', 'imt-dock-host', 'imt-status-bar', 'imt-command-palette']) {
      expect(fixture.nativeElement.querySelector(tag), tag).not.toBeNull();
    }
    // Only the core views: the inspector waits auto-hidden on the right; metrics, the profile editor, analysis and
    // tool windows open on demand.
    expect([...dock.openIds()].sort()).toEqual(['events', 'inspector', 'timeline']);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(fixture.nativeElement.querySelector('imt-timeline-panel')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-inspector')).toBeNull();
    expect(fixture.nativeElement.querySelector('imt-events-grid')).not.toBeNull();
  });

  it('shows the welcome over the dock when the workspace is empty', async () => {
    const { fixture, store } = await render();
    expect(fixture.nativeElement.querySelector('imt-welcome')).toBeNull();
    await store.clearWorkspace();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-welcome')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('imt-dock-host')).not.toBeNull();
    // Nothing to filter yet: the welcome page is the whole first screen.
    expect(fixture.nativeElement.querySelector('imt-filter-bar')).toBeNull();
    expect(fixture.nativeElement.querySelector('imt-fields-panel')).toBeNull();
    expect(fixture.nativeElement.querySelector('[data-testid=folder-setup-hint]')).toBeNull();
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
    // In the toast stack, one above the other.
    const stack = document.querySelector('.imt-toast-stack')!;
    expect(stack.querySelector('.toast[role=alert]')?.textContent).toContain('Something failed');
    expect(stack.querySelector('.toast[role=status]')?.textContent).toContain('Exported 3 events');
    expect(text(fixture)).toContain('Drop files or folders');
    stack.querySelector<HTMLButtonElement>('.toast[role=alert] button')!.click();
    stack.querySelector<HTMLButtonElement>('.toast[role=status] button')!.click();
    await settle(fixture);
    expect(stack.querySelector('.toast')).toBeNull();
    fixture.destroy();
    expect(document.querySelector('imt-workspace-toasts')).toBeNull();
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
