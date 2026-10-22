import { Component, Provider, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackendBare, settle, testLogger } from '../../testing/helpers';
import { ActivityItems, activityItemProblem, viewCategoryMenu } from './activity-items';
import { CommandRegistry } from './command-registry';
import { provideImtPlugin } from './provide';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_VIEWS, ImtActivityItem, ImtCommand, ImtView } from './tokens';

@Component({ selector: 'imt-test-view', template: '' })
class TestView {}

const view = (id: string, patch: Partial<ImtView> = {}): ImtView => ({ id, title: id.toUpperCase(), icon: 'chart', component: async () => TestView, ...patch });
const item = (id: string, patch: Partial<ImtActivityItem> = {}): ImtActivityItem => ({ id, label: id, icon: 'chart', order: 100, run: { command: 'test.run' }, ...patch });

function setup(items: unknown[], extra: { views?: ImtView[]; commands?: ImtCommand[]; providers?: Provider[] } = {}) {
  const dock = new FakeDock();
  const ran: string[] = [];
  const commands: ImtCommand[] = [{ id: 'test.run', title: 'Run test', keybinding: 'Mod+J', run: () => void ran.push('test.run') }, ...(extra.commands ?? [])];
  TestBed.configureTestingModule({
    providers: [
      ...provideTinyBackendBare(),
      provideFakeDock(dock),
      { provide: IMT_VIEWS, multi: true, useValue: extra.views ?? [] },
      { provide: IMT_COMMANDS, multi: true, useValue: commands },
      { provide: IMT_ACTIVITY_ITEMS, multi: true, useValue: items },
      ...(extra.providers ?? []),
    ],
  });
  return { registry: TestBed.inject(ActivityItems), dock, ran };
}

describe('ActivityItems', () => {
  it('sorts by order, keeping registration order for equal orders, and splits the sections', () => {
    const { registry } = setup([item('c', { order: 300 }), item('a', { order: 100 }), item('s', { order: 50, section: 'bottom' }), item('b', { order: 100 })]);
    expect(registry.all.map((i) => i.id)).toEqual(['s', 'a', 'b', 'c']);
    expect(registry.top().map((i) => i.id)).toEqual(['a', 'b', 'c']);
    expect(registry.bottom().map((i) => i.id)).toEqual(['s']);
  });

  it('keeps the first of duplicate ids and skips malformed contributions, logging each', () => {
    const { registry } = setup([item('a', { label: 'first' }), item('a', { label: 'second' }), { id: 'x' }, null, item('n', { order: Number.NaN })]);
    expect(registry.all.map((i) => i.label)).toEqual(['first']);
    const warnings = testLogger().entries.filter((e) => e.category === 'ui.activity');
    expect(warnings.map((e) => e.message)).toEqual(['Duplicate activity item skipped', 'Activity item skipped', 'Activity item skipped', 'Activity item skipped']);
  });

  it('describes what is wrong with a contribution', () => {
    expect(activityItemProblem('x')).toBe('not an object');
    expect(activityItemProblem({ ...item('a'), id: '' })).toBe('missing id');
    expect(activityItemProblem({ ...item('a'), label: 3 })).toBe('missing label');
    expect(activityItemProblem({ ...item('a'), icon: undefined })).toBe('missing icon');
    expect(activityItemProblem({ ...item('a'), section: 'left' })).toBe('unknown section');
    expect(activityItemProblem({ ...item('a'), run: { view: '' } })).toBe('run is not a view, a command or a menu');
    expect(activityItemProblem({ ...item('a'), run: { menu: [] } })).toBe('run is not a view, a command or a menu');
    expect(activityItemProblem(item('a', { run: { menu: () => [] } }))).toBeNull();
  });

  it('shows an item only while its when holds, and a view item only while the view is offered', () => {
    const on = signal(false);
    const enabled = signal(true);
    const { registry } = setup([item('w', { when: () => on() }), item('v', { run: { view: 'v1' } }), item('gone', { run: { view: 'missing' } })], {
      views: [view('v1', { enabled: () => enabled() })],
    });
    expect(registry.visible().map((i) => i.id)).toEqual(['v']);
    on.set(true);
    expect(registry.visible().map((i) => i.id)).toEqual(['w', 'v']);
    enabled.set(false);
    expect(registry.visible().map((i) => i.id)).toEqual(['w']);
  });

  it('formats badges: numbers capped at 99+, short text, none for null or empty', () => {
    const value = signal<string | number | null>(3);
    const { registry } = setup([item('b', { badge: () => value() })]);
    const b = registry.all[0]!;
    expect(registry.badge(b)).toBe('3');
    value.set(250);
    expect(registry.badge(b)).toBe('99+');
    value.set('new!!');
    expect(registry.badge(b)).toBe('new!');
    value.set('');
    expect(registry.badge(b)).toBeNull();
    value.set(null);
    expect(registry.badge(b)).toBeNull();
    expect(registry.badge(item('none'))).toBeNull();
  });

  it('isolates throwing functions: the item hides or loses the extra, the others stay, and it logs once', () => {
    const boom = () => {
      throw new Error('plugin bug');
    };
    const { registry } = setup([item('bad-when', { when: boom }), item('bad-extras', { badge: boom, active: boom, pulse: boom, disabled: boom }), item('bad-menu', { run: { menu: boom } }), item('ok')]);
    expect(registry.visible().map((i) => i.id)).toEqual(['bad-extras', 'ok']);
    const extras = registry.all[1]!;
    expect([registry.badge(extras), registry.isActive(extras), registry.pulse(extras), registry.disabledReason(extras)]).toEqual([null, false, false, null]);
    registry.badge(extras);
    registry.visible();
    const failures = testLogger().entries.filter((e) => e.message === 'Activity item failed');
    expect(failures.map((e) => `${e.props?.['Item']}:${e.props?.['Part']}`).sort()).toEqual(['bad-extras:active', 'bad-extras:badge', 'bad-extras:disabled', 'bad-extras:pulse', 'bad-menu:menu', 'bad-when:when']);
  });

  it('is active by its own function, else while its view is open', () => {
    const { registry, dock } = setup([item('own', { active: () => true }), item('v', { run: { view: 'v1' } }), item('cmd')], { views: [view('v1')] });
    const [own, v, cmd] = registry.all;
    expect(registry.isActive(own!)).toBe(true);
    expect(registry.isActive(v!)).toBe(false);
    dock.openIds.set(new Set(['v1']));
    expect(registry.isActive(v!)).toBe(true);
    expect(registry.isActive(cmd!)).toBe(false);
  });

  it('reads the key binding from the item, its command, or its view toggle command', () => {
    const { registry } = setup([item('own', { keybinding: '/' }), item('cmd'), item('v', { run: { view: 'v1' } }), item('menu', { run: { menu: () => [] } })], {
      views: [view('v1')],
      commands: [{ id: 'view.open.v1', title: 'Open v1', keybinding: 'Mod+1', run: () => undefined }],
    });
    expect(registry.all.map((i) => registry.keybinding(i))).toEqual(['/', 'Mod+J', 'Mod+1', null]);
  });

  it('lists only well-formed, unique, runnable menu entries and hides an empty fly-out', () => {
    const entries = signal([
      { id: 'a', label: 'A', run: { view: 'v1' } },
      { id: 'a', label: 'A again', run: { view: 'v1' } },
      { id: 'b', label: 'B', run: { view: 'missing' } },
      { id: 'c', label: 'C', run: { command: 'test.run' } },
      { id: 'd', label: 'D', run: { command: 'test.off' } },
      { id: 'e', label: 'E', run: {} },
    ]);
    const { registry } = setup([item('menu', { run: { menu: () => entries() as never } })], {
      views: [view('v1')],
      commands: [{ id: 'test.off', title: 'Off', when: () => false, run: () => undefined }],
    });
    const menu = registry.all[0]!;
    expect(registry.hasMenu(menu)).toBe(true);
    expect(registry.menu(menu).map((e) => e.label)).toEqual(['A', 'C']);
    expect(registry.visible().length).toBe(1);
    entries.set([]);
    expect(registry.visible().length).toBe(0);
    expect(registry.menu(item('plain'))).toEqual([]);
  });

  it('runs a view target through the dock and a command target through the command registry', async () => {
    const { registry, dock, ran } = setup([]);
    registry.run({ view: 'v1' });
    registry.run({ command: 'test.run' });
    await settle();
    expect(dock.calls).toEqual(['show:v1']);
    expect(ran).toEqual(['test.run']);
  });

  it('accepts items from a plugin registered with provideImtPlugin', () => {
    const { registry } = setup([], {
      providers: [provideImtPlugin({ commands: [{ id: 'hello.say', title: 'Say hello', run: () => undefined }], activityItems: [item('hello', { label: 'Hello', order: 450, run: { command: 'hello.say' } })] }) as never],
    });
    expect(registry.top().map((i) => i.label)).toEqual(['Hello']);
    expect(TestBed.inject(CommandRegistry).find('hello.say')).toBeTruthy();
  });
});

describe('viewCategoryMenu', () => {
  it('lists the offered views of a category (listed ones first), with check marks, then the available commands', () => {
    const { dock } = setup([], {
      views: [
        view('b', { category: 'Tools' }),
        view('a', { category: 'Tools' }),
        view('hidden', { category: 'Tools', enabled: () => false }),
        view('granular', { category: 'Tools', menu: 'palette' }),
        view('other', { category: 'Analysis' }),
      ],
      commands: [{ id: 'tools.collect', title: 'Collect…', run: () => undefined }],
    });
    dock.openIds.set(new Set(['a']));
    const menu = TestBed.runInInjectionContext(() => viewCategoryMenu('Tools', { first: ['a', 'missing'], commands: ['tools.collect', 'not.registered'] }));
    expect(menu()).toEqual([
      { id: 'view:a', label: 'A', icon: 'chart', checked: true, run: { view: 'a' } },
      { id: 'view:b', label: 'B', icon: 'chart', checked: false, run: { view: 'b' } },
      { id: 'command:tools.collect', label: 'Collect…', run: { command: 'tools.collect' } },
    ]);
    const plugins = TestBed.runInInjectionContext(() => viewCategoryMenu('Plugins'));
    expect(plugins()).toEqual([]);
  });
});
