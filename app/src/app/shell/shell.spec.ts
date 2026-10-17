import { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { TimelineZoom } from '../timeline/zoom';
import { ActivityBar } from './activity-bar';
import { EditorTabs } from './editor-tabs';
import { Sidebar } from './sidebar';
import { StatusBar } from './status-bar';
import { TitleBar } from './title-bar';

const fakePlatform = {
  isTauri: () => false,
  pickPaths: async (dir: boolean) => [dir ? '/picked/dir' : '/picked/file.log'],
  onFilesDropped: async () => () => undefined,
  pickProfileToOpen: async () => null,
  pickProfileToSave: async () => null,
};

function setup<T>(cmp: Type<T>, backend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: Platform, useValue: fakePlatform }] });
  const fixture = TestBed.createComponent(cmp);
  return { fixture, store: TestBed.inject(WorkspaceStore), backend };
}

describe('TitleBar', () => {
  it('runs menu commands', async () => {
    const { fixture, store } = setup(TitleBar);
    await settle(fixture);
    expect(text(fixture)).toContain('Indexed');
    const menu = (label: string, item: string) => {
      buttonByText(fixture, label).click();
      fixture.detectChanges();
      buttonByText(fixture, new RegExp('^' + item.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))).click();
      fixture.detectChanges();
    };
    menu('View', 'Toggle sidebar');
    expect(store.sidebarOpen()).toBe(false);
    menu('View', 'Toggle inspector');
    expect(store.inspectorOpen()).toBe(false);
    menu('View', 'Metrics');
    expect(store.activeTab()).toBe('metrics');
    menu('View', 'Profile editor');
    expect(store.activeTab()).toBe('profile');
    menu('View', 'Timeline');
    menu('View', 'Theme: light');
    expect(store.theme()).toBe('light');
    menu('View', 'Theme: dark');
    menu('View', 'Theme: system');
    menu('Analyze', 'Marked events only');
    expect(store.markedOnly()).toBe(true);
    menu('Analyze', 'Clear all filters');
    expect(store.markedOnly()).toBe(false);
    menu('Analyze', 'Time in local zone');
    expect(store.timezone()).toBe('local');
    menu('Analyze', 'Time in UTC');
    menu('Analyze', 'Search events');
    expect(store.focusSearchNonce()).toBe(1);
    TestBed.inject(TimelineZoom).zoomIn();
    menu('Analyze', 'Previous time range');
    expect(store.hasRange()).toBe(false);
    menu('Analyze', 'Next time range');
    expect(store.hasRange()).toBe(true);
    menu('Analyze', 'Fit time range');
    expect(store.hasRange()).toBe(false);
    menu('Help', 'IMT 0.1.0');
    menu('File', 'Open files…');
    await settle(fixture);
    expect(store.summary()?.sources.some((s) => s.path === '/picked/file.log')).toBe(true);
    menu('File', 'Open folder…');
    await settle(fixture);
    menu('File', 'Re-import all');
    await settle(fixture);
    menu('File', 'Close workspace');
    await settle(fixture);
    expect(store.summary()?.total).toBe(0);
    buttonByText(fixture, 'File').click();
    buttonByText(fixture, 'File').click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.dropdown')).toBeNull();
  });

  it('focuses search with / unless typing, and opens it from the palette', async () => {
    const { fixture, store } = setup(TitleBar);
    await settle(fixture);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/' }));
    expect(store.focusSearchNonce()).toBe(1);
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
    expect(store.focusSearchNonce()).toBe(1);
    input.remove();
    el<HTMLButtonElement>(fixture, '.palette').click();
    expect(store.focusSearchNonce()).toBe(2);
  });

  it('shows busy state', async () => {
    const { fixture } = setup(TitleBar);
    fixture.detectChanges();
    expect(text(fixture)).toContain('Loading workspace');
  });
});

describe('ActivityBar', () => {
  it('switches views and settings', async () => {
    const { fixture, store } = setup(ActivityBar);
    await settle(fixture);
    const click = (label: string) => el<HTMLButtonElement>(fixture, `[aria-label="${label}"]`).click();
    click('Metrics');
    expect(store.activeTab()).toBe('metrics');
    click('Profile editor');
    expect(store.activeTab()).toBe('profile');
    click('Timeline');
    expect(store.activeTab()).toBe('timeline');
    click('Search');
    expect(store.focusSearchNonce()).toBe(1);
    click('Explorer');
    expect(store.sidebarOpen()).toBe(false);
    click('Settings');
    await settle(fixture);
    buttonByText(fixture, 'light').click();
    expect(store.theme()).toBe('light');
    buttonByText(fixture, 'Local').click();
    expect(store.timezone()).toBe('local');
    buttonByText(fixture, 'UTC').click();
    el<HTMLInputElement>(fixture, '.panel input[type=checkbox]').click();
    expect(store.inspectorOpen()).toBe(false);
    document.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.panel')).toBeNull();
  });
});

describe('StatusBar', () => {
  it('summarises the workspace', async () => {
    const { fixture, store } = setup(StatusBar);
    fixture.detectChanges();
    expect(text(fixture)).toContain('No workspace');
    await settle(fixture);
    store.setMatchTotal(3);
    await settle(fixture);
    expect(text(fixture)).toContain('2 sources');
    expect(text(fixture)).toContain('11 events');
    expect(text(fixture)).toContain('3 matching');
    expect(text(fixture)).toContain('Importers: json · text');
    el<HTMLButtonElement>(fixture, '.tz').click();
    expect(store.timezone()).toBe('local');
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('Importers: —');
    expect(text(fixture)).toContain('0 sources');
  });

  it('pluralises one source', async () => {
    const { fixture, store } = setup(StatusBar);
    await settle(fixture);
    await store.removeSource(1);
    await settle(fixture);
    expect(el(fixture, '.strong').textContent?.trim()).toBe('1 source');
  });
});

describe('EditorTabs', () => {
  it('switches tabs', async () => {
    const { fixture, store } = setup(EditorTabs);
    await settle(fixture);
    all<HTMLButtonElement>(fixture, '.tab')[1].click();
    expect(store.activeTab()).toBe('metrics');
    await settle(fixture);
    expect(all(fixture, '.tab')[1].getAttribute('aria-selected')).toBe('true');
  });
});

describe('Sidebar', () => {
  it('lists sources and unrecognised files and manages them', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    expect(all(fixture, '.src:not(.dim) .name').map((n) => n.textContent)).toEqual(['web01/app.clef', 'web02/app.log']);
    expect(el(fixture, '.src.dim').textContent).toContain('system/export.reg');
    expect(el(fixture, '.src').getAttribute('title')).toContain('7 events');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove app.log"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
    el<HTMLButtonElement>(fixture, '[aria-label="Open files"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Open folder"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(3);
    el<HTMLButtonElement>(fixture, '[aria-label="Re-import all"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(2);
  });

  it('shows an empty state with open buttons', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('No files open');
    buttonByText(fixture, 'Open folder…').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
    await store.clearWorkspace();
    await settle(fixture);
    buttonByText(fixture, 'Open files…').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
  });

  it('shows source errors', async () => {
    const backend = tinyBackend();
    const original = backend.workspaceSummary.bind(backend);
    vi.spyOn(backend, 'workspaceSummary').mockImplementation(async () => {
      const s = await original();
      s.sources[0].error = 'Permission denied';
      return s;
    });
    const { fixture } = setup(Sidebar, backend);
    await settle(fixture);
    expect(el(fixture, '.src-error').textContent).toBe('Permission denied');
  });

  it('filters fields and opens facets with actions', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input.filter'), 'machine');
    await settle(fixture);
    const fields = all<HTMLButtonElement>(fixture, '.field');
    expect(fields).toHaveLength(1);
    expect(fields[0].classList.contains('grouped')).toBe(true);
    fields[0].click();
    await settle(fixture);
    expect(text(fixture)).toContain('2 values in current results');
    expect(all(fixture, '.value')).toHaveLength(2);
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName equals WEB01"]').click();
    expect(store.query().filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.facet')).toBeNull();

    fields[0].click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName not equal WEB01"]').click();
    fields[0].click();
    await settle(fixture);
    buttonByText(fixture, 'Group by').click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName']);
    fields[0].click();
    await settle(fixture);
    buttonByText(fixture, 'Add to group').click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName']);
    fields[0].click();
    await settle(fixture);
    buttonByText(fixture, 'Add column').click();
    expect(store.extraColumns()).toContain('MachineName');
    fields[0].click();
    await settle(fixture);
    fields[0].click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.facet')).toBeNull();
  });

  it('marks derived fields and handles facet errors', async () => {
    const backend = tinyBackend();
    const { fixture, store } = setup(Sidebar, backend);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input.filter'), 'cpu');
    await settle(fixture);
    expect(el(fixture, '.field .badge').textContent).toBe('fx');
    vi.spyOn(backend, 'queryFacet').mockRejectedValueOnce('facet failed');
    el<HTMLButtonElement>(fixture, '.field').click();
    await settle(fixture);
    expect(store.error()).toBe('facet failed');
    expect(text(fixture)).toContain('Loading');
  });
});

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input'));
}
