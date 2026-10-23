import { buildMenus, menuItems, recentLabel, RECENT_MENU_FILES, RECENT_MENU_PROJECTS, MenuNode, MenuState, MenuView, tidy, TopMenu, topLevelSize } from './menu-model';
import { at } from '../../../testing/must';

const COMMANDS: [string, string, string?][] = [
  ['project.new', 'New project'],
  ['window.new', 'New window', 'Mod+Shift+N'],
  ['file.openFiles', 'Open files…', 'Mod+O'],
  ['file.openFolder', 'Open folder…', 'Mod+Shift+O'],
  ['live.open', 'Open for live tail…'],
  ['profiling.open', 'Open performance capture…'],
  ['project.open', 'Open project…'],
  ['project.recent', 'Open recent project…'],
  ['project.save', 'Save project', 'Mod+S'],
  ['project.saveAs', 'Save project as…', 'Mod+Shift+S'],
  ['share.redactedCopy', 'Share redacted copy…'],
  ['file.reimport', 'Re-import all'],
  ['live.toggle', 'Toggle live tail'],
  ['file.export', 'Export events…', 'Mod+Shift+E'],
  ['file.close', 'Close workspace'],
  ['window.close', 'Close window'],
  ['edit.undo', 'Undo', 'Mod+Z'],
  ['edit.redo', 'Redo', 'Mod+Shift+Z'],
  ['edit.copy', 'Copy'],
  ['edit.copyJson', 'Copy as JSON'],
  ['edit.pasteLog', 'Paste as log…', 'Mod+Shift+V'],
  ['edit.find', 'Find', 'Mod+F'],
  ['palette.open', 'Show all commands', 'Mod+K'],
  ['view.toggleSidebar', 'Toggle sidebar', 'Mod+B'],
  ['view.resetLayout', 'Reset layout'],
  ['view.theme.system', 'Theme: Follow system'],
  ['view.theme.calm', 'Theme: Calm'],
  ['view.theme.console', 'Theme: Console'],
  ['view.theme.precision', 'Theme: Precision'],
  ['view.theme.contrast', 'Theme: High contrast'],
  ['view.scheme.system', 'Colour scheme: system'],
  ['view.scheme.dark', 'Colour scheme: dark'],
  ['view.scheme.light', 'Colour scheme: light'],
  ['settings.open', 'Open settings', 'Mod+,'],
  ['analyze.search', 'Search events'],
  ['query.focus', 'Query: focus query bar', 'Mod+/'],
  ['analyze.markedOnly', 'Marked events only'],
  ['analyze.clearFilters', 'Clear all filters'],
  ['analyze.fit', 'Fit time range'],
  ['analyze.back', 'Previous time range'],
  ['analyze.forward', 'Next time range'],
  ['nav.back', 'Go back', 'Alt+Left'],
  ['nav.forward', 'Go forward', 'Alt+Right'],
  ['analyze.utc', 'Time in UTC'],
  ['analyze.local', 'Time in local zone'],
  ['analyze.toggleUtc', 'Toggle UTC / local time'],
  ['moment.analyse', 'Analyse this moment', 'Mod+Shift+M'],
  ['compare.previous', 'Compare: current range vs previous'],
  ['fieldstats.show', 'Field statistics…'],
  ['ai.query', 'AI: Ask AI to write a query…'],
  ['ai.setup', 'Set up AI assistant'],
  ['ai.models', 'AI: Manage models…'],
  ['ai.explain', 'AI: Explain the selected event'],
  ['live.rules', 'Edit live alert rules…'],
  ['report.new', 'New incident report'],
  ['incident.build', 'Report: Incident summary…'],
  ['report.addTimeline', 'Add timeline to report'],
  ['notes.addSelected', 'Add note to the selected event…', 'Mod+D'],
  ['skew.adjust', "Adjust clock of the selected event's source…"],
  ['tools.collect', 'Collect diagnostics bundle…'],
  ['diagnostics.capture', 'Capture diagnostic screenshot', 'Mod+Shift+F12'],
  ['parsers.manage', 'Manage parsers…'],
  ['app.restartSafeMode', 'Restart in safe mode'],
  ['window.layout.save', 'Save current as perspective…'],
  ['window.perspective.1', 'Perspective: Investigate', 'Mod+1'],
  ['window.perspective.3', 'Perspective: Live', 'Mod+3'],
  ['window.layout.manage', 'Manage perspectives…'],
  ['window.layout.reset', 'Reset perspective'],
  ['window.autoHideAll', 'Auto hide all tool windows'],
  ['window.pinAll', 'Pin all tool windows'],
  ['window.closeAllDocuments', 'Close all documents'],
  ['window.float', 'Float panel'],
  ['window.popOut', 'Pop out to window', 'Ctrl+Alt+O'],
  ['window.fullScreen', 'Full screen', 'Shift+Alt+Enter'],
  ['window.panelMenu', 'Panel menu (window position)', 'Alt+-'],
  ['window.switcher', 'Switch panel (Ctrl+Tab)'],
  ['help.welcome', 'Show welcome'],
  ['help.demo', 'Try the demo'],
  ['help.tour', 'Take the tour'],
  ['help.shortcuts', 'Keyboard shortcuts', 'F1'],
  ['help.whatsNew', "What's new"],
  ['help.notices', 'Third-party notices'],
  ['help.about', 'About Debrief'],
];

const view = (id: string, title: string, category?: MenuView['category'], patch: Partial<MenuView> = {}): MenuView => ({ id, title, category, open: false, ...patch });

const VIEWS: MenuView[] = [
  view('timeline', 'Timeline', 'core', { open: true }),
  view('events', 'Events', 'core', { open: true }),
  view('inspector', 'Inspector', 'core', { open: true }),
  view('metrics', 'Metrics', 'core'),
  view('profile', 'Profile', 'core'),
  view('query', 'Query', 'core', { open: true }),
  view('home', 'Home', 'core'),
  view('plugins', 'Plugins', 'Plugins'),
  view('markers', 'Markers', 'Analysis'),
  ...(
    [
      ['query-results', 'Query results'],
      ['moment', 'Moment'],
      ['anomalies', 'Anomalies'],
      ['patterns', 'Patterns'],
      ['exceptions', 'Exceptions'],
      ['compare', 'Compare'],
      ['field-stats', 'Field statistics'],
      ['traces', 'Traces'],
    ] as const
  ).map(([id, t]) => view(id, t, 'Analysis')),
  view('live-alerts', 'Alerts', 'Live'),
  view('tail-status', 'Tail status', 'Live'),
  view('profiling.captures', 'Captures', 'Profiling'),
  view('profiling.hotspots', 'Hotspots', 'Profiling'),
  view('profiling.inverted', 'Inverted tree', 'Profiling', { menu: 'palette' }),
  view('profiling.butterfly', 'Callers and callees', 'Profiling', { menu: 'palette' }),
  view('perfmon', 'Performance Monitor', 'Tools'),
  view('report', 'Report', 'Sharing'),
  view('error-snapshots', 'Error snapshots', 'Diagnostics'),
  view('plugin-health', 'Plugin health', 'Diagnostics'),
  view('ai-chat', 'AI assistant', 'AI'),
  view('registry', 'Registry', undefined, { plugin: 'Windows registry' }),
  view('evtx', 'Event log', undefined, { plugin: 'Windows events' }),
  view('stray', 'Stray', undefined),
];

function state(patch: Partial<MenuState> = {}): MenuState {
  return {
    commands: new Map(COMMANDS.map(([id, title, keybinding]) => [id, { id, title, keybinding }])),
    views: VIEWS,
    sidebarOpen: true,
    theme: 'console',
    scheme: 'dark',
    timezone: 'utc',
    markedOnly: false,
    undoLabel: 'level filter change',
    redoLabel: null,
    windows: [{ label: 'main', title: 'Debrief', current: true }],
    popouts: [],
    nativeAppMenu: false,
    ...patch,
  };
}

const menu = (menus: TopMenu[], id: string) => menus.find((m) => m.id === id)!;
const labels = (nodes: readonly MenuNode[]) => nodes.map((n) => (n.kind === 'separator' ? '—' : n.label));
const sub = (nodes: readonly MenuNode[], label: string) => {
  const s = nodes.find((n) => n.kind === 'submenu' && n.label === label);
  if (!s || s.kind !== 'submenu') throw new Error(`no submenu ${label}`);
  return s.items;
};

describe('menu model', () => {
  it('builds the VS-style menu bar', () => {
    const menus = buildMenus(state());
    expect(menus.map((m) => m.label)).toEqual(['File', 'Edit', 'View', 'Analyze', 'Tools', 'Window', 'Help']);
  });

  it('puts the core views at the top of View, one submenu per category, then Other windows and Theme', () => {
    const v = menu(buildMenus(state()), 'view').items;
    expect(labels(v)).toEqual([
      'Explorer',
      'Fields',
      'Inspector',
      'Timeline',
      'Events',
      'Metrics',
      'Query',
      'Home',
      'Profile editor',
      '—',
      'Analysis',
      'Live',
      'Profiling',
      'Tools',
      'Sharing',
      'Diagnostics',
      'Plugins',
      'AI',
      '—',
      'Other windows…',
      '—',
      'Theme',
    ]);
    // The analyses in a fixed order; other analysis views (markers, notes, plugins') after them.
    expect(labels(sub(v, 'Analysis'))).toEqual(['Query results', 'Moment', 'Anomalies', 'Patterns', 'Exceptions', 'Compare', 'Field statistics', 'Traces', '—', 'Markers']);
    expect(labels(sub(v, 'Live'))).toEqual(['Toggle live tail', '—', 'Alerts', 'Tail status']);
    // Granular profiling views are reached from inside the profiling views (and the palette), not the menu.
    expect(labels(sub(v, 'Profiling'))).toEqual(['Captures', 'Hotspots']);
    expect(labels(sub(v, 'AI'))).toEqual(['Chat', 'Set up…', 'Models…']);
    // Check marks: open views and the sidebar; the theme as a choice.
    const items = menuItems(v);
    expect(items.find((i) => i.label === 'Explorer')).toMatchObject({ id: 'view.toggleSidebar', checked: true, keybinding: 'Mod+B' });
    expect(items.find((i) => i.label === 'Timeline')).toMatchObject({ id: 'view:timeline', checked: true });
    expect(items.find((i) => i.label === 'Metrics')).toMatchObject({ checked: false });
    expect(labels(sub(v, 'Theme'))).toEqual(['Follow system', 'Calm', 'Console', 'Precision', 'High contrast', '—', 'System', 'Dark', 'Light']);
    expect(sub(v, 'Theme').map((n) => n.kind === 'item' && n.checked)).toEqual([false, false, true, false, false, false, false, true, false]);
  });

  it('puts plugin views in Plugins, grouped by plugin, after the plugin manager', () => {
    const plugins = sub(menu(buildMenus(state()), 'view').items, 'Plugins');
    expect(labels(plugins)).toEqual(['Manage plugins', '—', 'Other plugins', 'Windows events', 'Windows registry']);
    const registry = plugins.find((n) => n.kind === 'group' && n.label === 'Windows registry');
    expect(registry?.kind === 'group' && registry.items.map((i) => i.id)).toEqual(['view:registry']);
  });

  it('File > Open recent lists projects, then files, then Clear recent; it is absent when nothing was opened (F5)', () => {
    const fileMenu = (s: MenuState) => menu(buildMenus(s), 'file').items;
    expect(labels(fileMenu(state()))).not.toContain('Open recent');
    const files = Array.from({ length: RECENT_MENU_FILES + 3 }, (_, i) => `C:\\logs\\web01\\app-${i}.log`);
    const projects = Array.from({ length: RECENT_MENU_PROJECTS + 2 }, (_, i) => ({ path: `/p/proj-${i}.debriefproj`, name: `Proj ${i}` }));
    const recent = sub(fileMenu(state({ recent: { projects, files } })), 'Open recent');
    const items = recent.map((n) => (n.kind === 'item' ? n.label : '—'));
    expect(items).toHaveLength(RECENT_MENU_PROJECTS + 1 + RECENT_MENU_FILES + 1 + 1);
    expect(items[0]).toBe('Proj 0');
    expect(items[RECENT_MENU_PROJECTS]).toBe('—');
    expect(items[RECENT_MENU_PROJECTS + 1]).toBe('web01/app-0.log');
    expect(items.at(-1)).toBe('Clear recent');
    expect(menuItems(recent).map((i) => i.id)).toContain('recent:project:/p/proj-0.debriefproj');
    expect(menuItems(recent).map((i) => i.id)).toContain('menu.recentClear');
    // Only files: no separator between the groups.
    const onlyFiles = sub(fileMenu(state({ recent: { projects: [], files: ['/a/b.log'] } })), 'Open recent');
    expect(onlyFiles.map((n) => (n.kind === 'item' ? n.label : '—'))).toEqual(['a/b.log', '—', 'Clear recent']);
    expect(recentLabel('single')).toBe('single');
    expect(recentLabel('')).toBe('');
  });

  it('leaves out commands that are not available, and menus and submenus that end up empty', () => {
    const only = (ids: string[]) => new Map([...state().commands].filter(([id]) => ids.includes(id)));
    const menus = buildMenus(state({ commands: only(['file.openFiles']), views: [view('timeline', 'Timeline', 'core')] }));
    expect(menus.map((m) => m.label)).toEqual(['File', 'View']);
    expect(labels(menu(menus, 'view').items)).toEqual(['Fields', 'Timeline', '—', 'Other windows…']);
  });

  it('never lists an action twice, and keeps palette-only commands and context actions out', () => {
    const menus = buildMenus(state());
    const ids = menus.flatMap((m) => menuItems(m.items).map((i) => i.id));
    expect(new Set(ids).size).toBe(ids.length);
    for (const demoted of ['palette.open', 'view.resetLayout', 'analyze.toggleUtc', 'ai.explain', 'notes.addSelected', 'skew.adjust', 'window.panelMenu', 'window.switcher']) {
      expect(ids, demoted).not.toContain(demoted);
    }
    expect(ids).toContain('window.layout.reset');
    expect(ids).toContain('window.perspective.1');
  });

  it('keeps every menu at most about 20 entries at the top level', () => {
    const many = state({ windows: Array.from({ length: 3 }, (_, i) => ({ label: `w-${i}`, title: `Debrief ${i}`, current: i === 0 })) });
    for (const m of buildMenus(many)) expect(topLevelSize(m.items), m.label).toBeLessThanOrEqual(22);
  });

  it('labels Undo and Redo with what they would take back', () => {
    const edit = menu(buildMenus(state()), 'edit').items;
    // Then Copy, Copy as JSON, Paste as log and Find (F4).
    expect(labels(edit)).toEqual(['Undo level filter change', 'Redo (nothing to redo)', '—', 'Copy', 'Copy as JSON', '—', 'Paste as log…', '—', 'Find']);
    expect(edit.find((n) => n.kind === 'item' && n.id === 'edit.find')).toMatchObject({ keybinding: 'Mod+F' });
  });

  it('groups Window: perspectives submenu, tool window actions, then the windows and pop-outs when there are several', () => {
    const w = menu(buildMenus(state()), 'window').items;
    expect(labels(w)).toEqual(['Perspectives', '—', 'Auto hide all tool windows', 'Pin all tool windows', 'Close all documents', '—', 'Float panel', 'Pop out to window', 'Full screen']);
    expect(labels(sub(w, 'Perspectives'))).toEqual(['Perspective: Investigate', 'Perspective: Live', '—', 'Save current as perspective…', '—', 'Manage perspectives…', 'Reset perspective']);
    expect(menuItems(sub(w, 'Perspectives')).some((i) => i.checked)).toBe(false);
    const two = menu(
      buildMenus(
        state({
          windows: [
            { label: 'main', title: 'Debrief', current: true },
            { label: 'w-2', title: '', current: false },
          ],
          popouts: [{ id: 'inspector', title: 'Inspector' }],
        }),
      ),
      'window',
    ).items;
    const groups = two.filter((n) => n.kind === 'group');
    expect(groups.map((g) => g.kind === 'group' && g.items.map((i) => [i.id, i.label, i.checked]))).toEqual([
      [
        ['win:main', 'Debrief', true],
        ['win:w-2', 'w-2', false],
      ],
      [['popout:inspector', 'Inspector', undefined]],
    ]);
  });

  it('puts Go back / Go forward in Analyze beside the time range steps, with their keys', () => {
    const items = menuItems(menu(buildMenus(state()), 'analyze').items);
    const ids = items.map((i) => i.id);
    expect(ids.slice(ids.indexOf('analyze.fit'), ids.indexOf('analyze.fit') + 5)).toEqual(['analyze.fit', 'analyze.back', 'analyze.forward', 'nav.back', 'nav.forward']);
    expect(items.find((i) => i.id === 'nav.back')).toMatchObject({ label: 'Go back', keybinding: 'Alt+Left' });
    expect(items.find((i) => i.id === 'nav.forward')).toMatchObject({ label: 'Go forward', keybinding: 'Alt+Right' });
  });

  it('offers the incident summary where reports are created', () => {
    const analyze = menu(buildMenus(state()), 'analyze').items;
    expect(menuItems(analyze).map((i) => i.id)).toContain('report.new');
    expect(menuItems(sub(analyze, 'Add to report'))[0]).toMatchObject({ id: 'incident.build', label: 'Incident summary…' });
  });

  it('leaves Go back / Go forward out when they are unavailable', () => {
    const without = new Map([...state().commands].filter(([id]) => !id.startsWith('nav.')));
    const ids = menuItems(menu(buildMenus(state({ commands: without })), 'analyze').items).map((i) => i.id);
    expect(ids).not.toContain('nav.back');
    expect(ids).not.toContain('nav.forward');
  });

  it('checks the active perspective in Window > Perspectives', () => {
    const checked = (active?: number) =>
      menuItems(sub(menu(buildMenus(state({ activePerspective: active })), 'window').items, 'Perspectives'))
        .filter((i) => i.checked)
        .map((i) => i.id);
    expect(checked()).toEqual([]);
    expect(checked(1)).toEqual(['window.perspective.1']);
    expect(checked(3)).toEqual(['window.perspective.3']);
    // A perspective without a command (hidden, or beyond nine) checks nothing.
    expect(checked(2)).toEqual([]);
    expect(checked(0)).toEqual([]);
  });

  it('builds Help, with About and Check for updates in the app menu on macOS', () => {
    expect(labels(menu(buildMenus(state()), 'help').items)).toEqual([
      'Welcome',
      'Try the demo',
      'Take the tour',
      'Keyboard shortcuts',
      '—',
      "What's new",
      '—',
      'Restart in safe mode',
      '—',
      'Third-party notices',
      'About Debrief',
    ]);
    const updates = new Map([...state().commands, ['help.checkForUpdates', { id: 'help.checkForUpdates', title: 'Check for updates' }]]);
    const mac = buildMenus(state({ nativeAppMenu: true, commands: updates }));
    expect(mac[0]).toMatchObject({ id: 'app', label: 'Debrief' });
    expect(labels(at(mac, 0).items)).toEqual(['About Debrief', 'Check for updates…', '—', 'Settings…']);
    const ids = mac.slice(1).flatMap((m) => menuItems(m.items).map((i) => i.id));
    for (const id of ['help.about', 'help.checkForUpdates', 'settings.open']) expect(ids).not.toContain(id);
    expect(labels(menu(buildMenus(state()), 'tools').items)).toContain('Settings…');
  });

  it('quits Debrief from File ▸ Exit, or last in the app menu on macOS', () => {
    const commands = new Map([...state().commands, ['app.quit', { id: 'app.quit', title: 'Exit' }]]);
    const file = menu(buildMenus(state({ commands })), 'file').items;
    expect(labels(file).slice(-3)).toEqual(['Close window', '—', 'Exit']);
    const mac = buildMenus(state({ nativeAppMenu: true, commands }));
    expect(labels(at(mac, 0).items).at(-1)).toBe('Quit Debrief');
    expect(labels(menu(mac, 'file').items)).not.toContain('Exit');
  });

  it('tidies separators and empty submenus and groups', () => {
    const item = { kind: 'item', id: 'a', label: 'A' } as const;
    expect(
      labels(
        tidy([
          { kind: 'separator' },
          item,
          { kind: 'separator' },
          { kind: 'separator' },
          { kind: 'submenu', id: 's', label: 'Empty', items: [{ kind: 'separator' }] },
          { kind: 'group', label: 'G', items: [] },
          null,
          { kind: 'separator' },
        ]),
      ),
    ).toEqual(['A']);
  });
});

describe('View › Source colours', () => {
  const withColors = (sourceColors?: MenuState['sourceColors']) => {
    const s = state(sourceColors ? { sourceColors } : {});
    const commands = new Map(s.commands);
    for (const [id, title] of [
      ['view.sourceColors.source', 'Source colours: by file'],
      ['view.sourceColors.instance', 'Source colours: by instance'],
      ['view.sourceColors.off', 'Source colours: off'],
    ] as const)
      commands.set(id, { id, title, keybinding: undefined });
    return sub(menu(buildMenus({ ...s, commands }), 'view').items, 'Source colours');
  };

  it('offers by file, by instance and off as a choice, by file by default', () => {
    expect(labels(withColors())).toEqual(['By file', 'By instance', 'Off']);
    expect(withColors().map((n) => n.kind === 'item' && n.checked)).toEqual([true, false, false]);
    expect(withColors('instance').map((n) => n.kind === 'item' && n.checked)).toEqual([false, true, false]);
    expect(withColors('off').map((n) => n.kind === 'item' && n.checked)).toEqual([false, false, true]);
  });

  it('is left out when the commands are not available', () => {
    expect(() => sub(menu(buildMenus(state()), 'view').items, 'Source colours')).toThrow(/no submenu/);
  });
});
