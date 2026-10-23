import { vi } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { provideTimelines } from './provide-timelines';
import { defaultConfig, MAX_TIMELINES } from './timeline-config';
import { TimelineFakeDock } from './timeline-testing';
import { loadTimelines, saveTimelines, TIMELINES_KEY, TimelineViews } from './timeline-views';

describe('timeline settings storage', () => {
  beforeEach(() => localStorage.clear());

  it('round-trips configs and tolerates missing, corrupt or foreign data', () => {
    expect(loadTimelines()).toEqual([]);
    saveTimelines([{ ...defaultConfig('timeline#2'), title: 'Errors' }]);
    expect(loadTimelines()).toEqual([{ ...defaultConfig('timeline#2'), title: 'Errors' }]);
    localStorage.setItem(TIMELINES_KEY, '{nope');
    expect(loadTimelines()).toEqual([]);
    localStorage.setItem(TIMELINES_KEY, JSON.stringify({ version: 99, timelines: [{ id: 'timeline#2' }] }));
    expect(loadTimelines()).toEqual([]);
    localStorage.setItem(TIMELINES_KEY, JSON.stringify({ version: 1, timelines: [{ id: 'bad' }, { id: 'timeline#3' }] }));
    expect(loadTimelines().map((c) => c.id)).toEqual(['timeline#3']);
  });

  it('never throws when storage is unavailable', () => {
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    expect(() => saveTimelines([defaultConfig('timeline#2')])).not.toThrow();
    expect(loadTimelines()).toEqual([]);
    set.mockRestore();
    get.mockRestore();
  });
});

describe('TimelineViews', () => {
  let dock: TimelineFakeDock;

  function setup() {
    dock = new TimelineFakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideTimelines(), { provide: DockService, useValue: dock }] });
    return TestBed.inject(TimelineViews);
  }

  beforeEach(() => localStorage.clear());

  it('opens numbered timelines below the main one with default settings', () => {
    const views = setup();
    expect(views.open()).toBe('timeline#2');
    expect(views.open({ title: 'DB errors', scope: 'own', ql: 'level >= error' })).toBe('timeline#3');
    expect(dock.calls).toEqual(['beside:timeline#2:timeline:below', 'beside:timeline#3:timeline:below']);
    expect(views.config('timeline#2')).toEqual(defaultConfig('timeline#2'));
    expect(views.config('timeline#3')).toMatchObject({ title: 'DB errors', scope: 'own', ql: 'level >= error', sync: true });
  });

  it('drops settings of closed panels and reuses their number', () => {
    const views = setup();
    views.open();
    views.update('timeline#2', { title: 'Old' });
    dock.close('timeline#2');
    expect(views.open()).toBe('timeline#2');
    expect(views.config('timeline#2').title).toBe('Timeline 2');
    expect(views.configs()).toHaveLength(1);
  });

  it('refuses more than the limit with a notice', () => {
    const views = setup();
    for (let i = 0; i < MAX_TIMELINES; i++) expect(views.open()).not.toBeNull();
    expect(views.open()).toBeNull();
    expect(TestBed.inject(WorkspaceStore).notice()).toMatch(/At most 12 additional timelines/);
  });

  it('updates and renames, and a blank name restores the default', () => {
    const views = setup();
    views.update('timeline#5', { sync: false });
    expect(views.config('timeline#5')).toMatchObject({ id: 'timeline#5', sync: false, title: 'Timeline 5' });
    views.rename('timeline#5', '  WEB01 vs DB01  ');
    expect(views.config('timeline#5').title).toBe('WEB01 vs DB01');
    views.rename('timeline#5', '   ');
    expect(views.config('timeline#5').title).toBe('Timeline 5');
    expect(views.config('timeline#9')).toEqual(defaultConfig('timeline#9'));
  });

  it('persists settings to local storage and restores them on the next start', async () => {
    const views = setup();
    views.open({ title: 'Kept' });
    await settle();
    expect(loadTimelines()).toEqual([{ ...defaultConfig('timeline#2'), title: 'Kept' }]);
    TestBed.resetTestingModule();
    const again = setup();
    expect(again.config('timeline#2').title).toBe('Kept');
  });

  it('saves the open panels with projects and sessions and restores them', () => {
    const views = setup();
    const registry = TestBed.inject(UiStateRegistry);
    views.open({ title: 'A' });
    views.update('timeline#7', { title: 'not open' });
    const ui = registry.serialize();
    expect(ui['timelines']).toEqual({ version: 1, timelines: [{ ...defaultConfig('timeline#2'), title: 'A' }] });

    views.rename('timeline#2', 'changed');
    registry.restore(ui);
    expect(views.config('timeline#2').title).toBe('A');
    expect(views.configs().map((c) => c.id)).toEqual(['timeline#2']);
    registry.restore({ timelines: 'garbage' });
    expect(views.configs()).toEqual([]);
    views.open();
    registry.restore({});
    expect(views.configs()).toEqual([]);
  });

  it('contributes the "New timeline" command', async () => {
    setup();
    const command = TestBed.inject(DEBRIEF_COMMANDS)
      .flat()
      .find((c) => c.id === 'timeline.new');
    expect(command).toMatchObject({ title: 'New timeline', category: 'View' });
    await command!.run({} as never);
    expect(dock.calls).toEqual(['beside:timeline#2:timeline:below']);
  });
});

describe('ViewRegistry and timeline panels', () => {
  it('resolves an additional timeline panel id to the timeline view only', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const views = TestBed.inject(ViewRegistry);
    expect(views.find('timeline#3')).toBe(views.find('timeline'));
    expect(views.find('events#2')).toBeUndefined();
    expect(views.find('timeline#')).toBeUndefined();
    expect(views.find('#2')).toBeUndefined();
  });
});
