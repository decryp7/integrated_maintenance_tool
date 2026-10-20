import { ANALYSIS_MENU_ORDER } from '../../analysis/analysis-menu-order';
import { ViewCategory } from '../../extensions/tokens';

/**
 * The application menus as plain data, built by a pure function from what is registered (views, commands) and the
 * state that decides check marks. One model feeds both the in-app menu bar (Windows, Linux, browser) and the macOS
 * native menu bar, so they never disagree.
 *
 * Item ids are command ids, or one of the prefixed ids below for items that are not commands.
 */

/** A clickable item. `checked` set means it is a toggle (menuitemcheckbox). */
export interface MenuItemNode {
  kind: 'item';
  id: string;
  label: string;
  /** Key binding in command syntax (e.g. "Mod+Shift+O"), shown as the shortcut hint. */
  keybinding?: string;
  checked?: boolean;
}

export interface SubmenuNode {
  kind: 'submenu';
  id: string;
  label: string;
  items: MenuNode[];
}

export interface SeparatorNode {
  kind: 'separator';
}

/** A labelled section of items (e.g. the views of one plugin). */
export interface GroupNode {
  kind: 'group';
  label: string;
  items: MenuItemNode[];
}

export type MenuNode = MenuItemNode | SubmenuNode | SeparatorNode | GroupNode;

export interface TopMenu {
  id: string;
  label: string;
  items: MenuNode[];
}

/** Item ids that are not commands. */
export const MENU_IDS = {
  /** `view:<view id>` toggles a view. */
  view: 'view:',
  /** `win:<label>` focuses an app window. */
  window: 'win:',
  /** `popout:<view id>` brings a popped-out panel's window to the front. */
  popout: 'popout:',
  /** Opens the searchable view picker. */
  otherWindows: 'menu.otherWindows',
  /** Shows the sidebar and focuses the fields filter. */
  fields: 'menu.fields',
} as const;

/**
 * Commands that live in the macOS app menu (IMT ▸) in native mode instead of Help and Tools: the `app` menu of the
 * model, which the shell merges with the standard items (Services, Hide, Quit).
 */
export const APP_MENU_COMMANDS = ['help.about', 'help.checkForUpdates', 'settings.open'] as const;

export interface MenuCommand {
  id: string;
  title: string;
  keybinding?: string;
}

export interface MenuView {
  id: string;
  title: string;
  category?: ViewCategory;
  plugin?: string;
  menu?: 'palette';
  open: boolean;
}

export interface MenuWindow {
  label: string;
  title: string;
  current: boolean;
}

export interface MenuPopout {
  id: string;
  title: string;
}

/** Everything the menus depend on. Commands and views are only the AVAILABLE ones. */
export interface MenuState {
  commands: ReadonlyMap<string, MenuCommand>;
  views: readonly MenuView[];
  sidebarOpen: boolean;
  theme: 'system' | 'dark' | 'light';
  /** What the source colour marker stands for (View › Source colours); default by file. */
  sourceColors?: 'source' | 'instance' | 'off';
  timezone: 'utc' | 'local';
  markedOnly: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  windows: readonly MenuWindow[];
  popouts: readonly MenuPopout[];
  /** macOS native menu bar: About and Settings are in the app menu, not in Help/Tools. */
  nativeAppMenu: boolean;
}

/** Order of the core views at the top of the View menu (after Explorer and Fields). */
export const CORE_VIEW_ORDER = ['inspector', 'timeline', 'events', 'metrics', 'query', 'overview', 'profile'];

/** Order of the View submenus. */
export const VIEW_SUBMENUS: readonly Exclude<ViewCategory, 'core'>[] = ['Analysis', 'Live', 'Profiling', 'Tools', 'Sharing', 'Diagnostics', 'Plugins', 'AI'];

/** Views listed first in their submenu, in this order; the others follow after a separator. */
const SUBMENU_ORDER: Partial<Record<ViewCategory, readonly string[]>> = {
  Analysis: ANALYSIS_MENU_ORDER,
};

/** A submenu's views: the ones with a set order first, then the rest (in registration order). */
function ordered(category: ViewCategory, views: readonly MenuView[]): Part[] {
  const order = SUBMENU_ORDER[category];
  if (!order) return views.map(viewItem);
  const first = order.map((id) => views.find((v) => v.id === id)).filter((v): v is MenuView => !!v);
  return [...first.map(viewItem), SEP, ...views.filter((v) => !first.includes(v)).map(viewItem)];
}

/** Menu labels that differ from the view titles (the tab keeps its short title). */
const VIEW_LABELS: Readonly<Record<string, string>> = {
  profile: $localize`:@@shell.menu.view.profileEditor:Profile editor`,
  plugins: $localize`:@@shell.menu.view.managePlugins:Manage plugins`,
  'ai-chat': $localize`:@@shell.menu.view.chat:Chat`,
};

/** Display names of the View submenus (the category itself stays the key). */
const CATEGORY_LABELS: Readonly<Record<Exclude<ViewCategory, 'core'>, string>> = {
  Analysis: $localize`:View submenu@@shell.menu.view.category.analysis:Analysis`,
  Live: $localize`:View submenu@@shell.menu.view.category.live:Live`,
  Profiling: $localize`:View submenu@@shell.menu.view.category.profiling:Profiling`,
  Tools: $localize`:View submenu@@shell.menu.view.category.tools:Tools`,
  Sharing: $localize`:View submenu@@shell.menu.view.category.sharing:Sharing`,
  Diagnostics: $localize`:View submenu@@shell.menu.view.category.diagnostics:Diagnostics`,
  Plugins: $localize`:View submenu@@shell.menu.view.category.plugins:Plugins`,
  AI: $localize`:View submenu@@shell.menu.view.category.ai:AI`,
};

/** A separator; collapsed when leading, trailing or doubled. */
const SEP: SeparatorNode = { kind: 'separator' };

type Part = MenuNode | null | undefined | false;

/** Drops missing parts, empty submenus and groups, and leading, trailing and doubled separators. */
export function tidy(parts: readonly Part[]): MenuNode[] {
  const out: MenuNode[] = [];
  for (const p of parts) {
    if (!p) continue;
    if (p.kind === 'separator') {
      if (out.length && out[out.length - 1]?.kind !== 'separator') out.push(p);
    } else if (p.kind === 'submenu') {
      const items = tidy(p.items);
      if (items.length) out.push({ ...p, items });
    } else if (p.kind === 'group') {
      if (p.items.length) out.push(p);
    } else out.push(p);
  }
  while (out.length && out[out.length - 1]?.kind === 'separator') out.pop();
  return out;
}

/** Builds the menu bar. Pure: same state, same menus. */
export function buildMenus(state: MenuState): TopMenu[] {
  const cmd = (id: string, label?: string, checked?: boolean): MenuItemNode | null => {
    const c = state.commands.get(id);
    if (!c) return null;
    return { kind: 'item', id, label: label ?? c.title, ...(c.keybinding ? { keybinding: c.keybinding } : {}), ...(checked === undefined ? {} : { checked }) };
  };
  const sub = (id: string, label: string, items: Part[]): SubmenuNode => ({ kind: 'submenu', id, label, items: tidy(items) });
  /** In the app menu in native mode, else where it is placed. */
  const appMenu = (id: (typeof APP_MENU_COMMANDS)[number], label?: string) => (state.nativeAppMenu ? null : cmd(id, label));
  const undo = (what: string | null) =>
    cmd('edit.undo', what ? $localize`:@@shell.menu.edit.undo:Undo ${what}:what:` : $localize`:@@shell.menu.edit.undoNothing:Undo (nothing to undo)`);
  const redo = (what: string | null) =>
    cmd('edit.redo', what ? $localize`:@@shell.menu.edit.redo:Redo ${what}:what:` : $localize`:@@shell.menu.edit.redoNothing:Redo (nothing to redo)`);

  const menus: TopMenu[] = [
    ...(state.nativeAppMenu
      ? [
          {
            id: 'app',
            label: 'IMT',
            items: tidy([
              cmd('help.about', $localize`:@@shell.menu.help.about:About IMT`),
              cmd('help.checkForUpdates', $localize`:@@shell.menu.help.checkForUpdates:Check for updates…`),
              SEP,
              cmd('settings.open', $localize`:@@shell.menu.settings:Settings…`),
              SEP,
              cmd('app.quit', $localize`:@@shell.menu.app.quit:Quit IMT`),
            ]),
          },
        ]
      : []),
    {
      id: 'file',
      label: $localize`:Top-level menu@@shell.menu.top.file:File`,
      items: tidy([
        cmd('project.new'),
        cmd('window.new'),
        SEP,
        cmd('file.openFiles'),
        cmd('file.openFolder'),
        cmd('live.open'),
        cmd('profiling.open'),
        cmd('project.open'),
        cmd('project.recent', $localize`:@@shell.menu.file.openRecentProject:Open recent project…`),
        SEP,
        cmd('project.save'),
        cmd('project.saveAs'),
        SEP,
        cmd('file.reimport'),
        cmd('folder.setupAnalysis'),
        cmd('live.toggle'),
        SEP,
        cmd('file.export'),
        cmd('report.export'),
        SEP,
        cmd('file.close'),
        cmd('window.close'),
        SEP,
        // On macOS Quit is in the app menu.
        state.nativeAppMenu ? null : cmd('app.quit'),
      ]),
    },
    {
      id: 'edit',
      label: $localize`:Top-level menu@@shell.menu.top.edit:Edit`,
      items: tidy([undo(state.undoLabel), redo(state.redoLabel)]),
    },
    { id: 'view', label: $localize`:Top-level menu@@shell.menu.top.view:View`, items: viewMenu(state, cmd, sub) },
    {
      id: 'analyze',
      label: $localize`:Top-level menu@@shell.menu.top.analyze:Analyze`,
      items: tidy([
        cmd('analyze.search'),
        cmd('query.focus', $localize`:@@shell.menu.analyze.focusQueryBar:Focus query bar`),
        cmd('markers.highlight'),
        cmd('analyze.markedOnly', $localize`:@@shell.menu.analyze.markedOnly:Marked events only`, state.markedOnly),
        cmd('analyze.clearFilters'),
        SEP,
        cmd('analyze.fit'),
        cmd('analyze.back'),
        cmd('analyze.forward'),
        sub('analyze.timezone', $localize`:@@shell.menu.analyze.timeZone:Time zone`, [
          cmd('analyze.utc', $localize`:@@shell.menu.analyze.utc:UTC`, state.timezone === 'utc'),
          cmd('analyze.local', $localize`:Time zone option@@shell.menu.analyze.local:Local`, state.timezone === 'local'),
        ]),
        SEP,
        cmd('moment.analyse'),
        cmd('moment.selected', $localize`:@@shell.menu.analyze.analyseSelectedRange:Analyse the selected range`),
        sub('analyze.compare', $localize`:Menu@@shell.menu.analyze.compare:Compare`, [
          cmd('compare.previous', $localize`:@@shell.menu.analyze.compare.previous:Current range vs previous`),
          cmd('compare.beforeAfter', $localize`:@@shell.menu.analyze.compare.beforeAfter:Before / after selected time`),
        ]),
        cmd('fieldstats.show'),
        SEP,
        sub('analyze.ai', $localize`:@@shell.menu.analyze.ai:AI`, [
          cmd('ai.query', $localize`:@@shell.menu.analyze.ai.query:Ask AI to write a query…`),
          cmd('ai.summarize', $localize`:@@shell.menu.analyze.ai.summarize:Summarize this range`),
          cmd('ai.rootCauses', $localize`:@@shell.menu.analyze.ai.rootCauses:Find likely root causes`),
        ]),
        SEP,
        cmd('live.resume'),
        cmd('live.rules'),
        SEP,
        cmd('report.new'),
        sub('analyze.report', $localize`:@@shell.menu.analyze.addToReport:Add to report`, [
          cmd('report.addSelected', $localize`:@@shell.menu.report.selectedEvent:Selected event`),
          cmd('report.addBookmarks', $localize`:@@shell.menu.report.bookmarks:Bookmarks`),
          cmd('report.addTimeline', $localize`:@@shell.menu.report.timeline:Timeline`),
          cmd('report.addAnnotations', $localize`:@@shell.menu.report.annotations:Annotations`),
          cmd('report.addQueryTable', $localize`:@@shell.menu.report.queryResult:Query result`),
          cmd('report.addFindings', $localize`:@@shell.menu.report.findings:Health-check findings`),
          cmd('report.addAnomalies', $localize`:@@shell.menu.report.anomalies:Anomalies as findings`),
        ]),
      ]),
    },
    {
      id: 'tools',
      label: $localize`:Top-level menu@@shell.menu.top.tools:Tools`,
      items: tidy([
        cmd('tools.collect'),
        cmd('diagnostics.capture'),
        SEP,
        cmd('profile.newFromTemplate'),
        SEP,
        cmd('app.restartElevated'),
        SEP,
        appMenu('settings.open', $localize`:@@shell.menu.settings:Settings…`),
      ]),
    },
    { id: 'window', label: $localize`:Top-level menu@@shell.menu.top.window:Window`, items: windowMenu(state, cmd, sub) },
    {
      id: 'help',
      label: $localize`:Top-level menu@@shell.menu.top.help:Help`,
      items: tidy([
        cmd('help.welcome', $localize`:@@shell.menu.help.welcome:Welcome`),
        cmd('help.demo'),
        cmd('help.tour'),
        cmd('help.shortcuts'),
        SEP,
        // Commands that are not registered (yet) are left out: these appear when their feature lands.
        cmd('help.documentation', $localize`:@@shell.menu.help.documentation:Documentation`),
        cmd('help.whatsNew', $localize`:@@shell.menu.help.whatsNew:What's new`),
        SEP,
        cmd('help.reportProblem'),
        cmd('help.activityLog'),
        cmd('help.openLogs', $localize`:@@shell.menu.help.openLogs:Open logs folder`),
        cmd('help.clearCache', $localize`:@@shell.menu.help.clearCache:Clear cache…`),
        cmd('app.restartSafeMode'),
        cmd('app.restartNormally'),
        SEP,
        appMenu('help.checkForUpdates', $localize`:@@shell.menu.help.checkForUpdates:Check for updates…`),
        cmd('help.notices', $localize`:@@shell.menu.help.notices:Third-party notices`),
        appMenu('help.about', $localize`:@@shell.menu.help.about:About IMT`),
      ]),
    },
  ];
  return menus.filter((m) => m.items.length);
}

type Cmd = (id: string, label?: string, checked?: boolean) => MenuItemNode | null;
type Sub = (id: string, label: string, items: Part[]) => SubmenuNode;

function viewItem(v: MenuView): MenuItemNode {
  return { kind: 'item', id: MENU_IDS.view + v.id, label: VIEW_LABELS[v.id] ?? v.title, checked: v.open };
}

/** Core tool windows at the top, then one submenu per category, then the picker for everything else. */
function viewMenu(state: MenuState, cmd: Cmd, sub: Sub): MenuNode[] {
  const listed = state.views.filter((v) => v.menu !== 'palette');
  const core = CORE_VIEW_ORDER.map((id) => listed.find((v) => v.id === id && v.category === 'core')).filter((v): v is MenuView => !!v);
  const extraCore = listed.filter((v) => v.category === 'core' && !CORE_VIEW_ORDER.includes(v.id));
  const inCategory = (c: ViewCategory) => listed.filter((v) => (v.category ?? 'Plugins') === c);
  const submenus = VIEW_SUBMENUS.map((c) => {
    const id = `view.${c.toLowerCase()}`;
    const label = CATEGORY_LABELS[c];
    if (c === 'Plugins') return sub(id, label, pluginItems(inCategory(c)));
    if (c === 'AI') {
      return sub(id, label, [
        ...inCategory(c).map(viewItem),
        cmd('ai.setup', $localize`:@@shell.menu.view.ai.setup:Set up…`),
        cmd('ai.models', $localize`:@@shell.menu.view.ai.models:Models…`),
      ]);
    }
    return sub(id, label, ordered(c, inCategory(c)));
  });
  return tidy([
    cmd('view.toggleSidebar', $localize`:@@shell.menu.view.explorer:Explorer`, state.sidebarOpen),
    { kind: 'item', id: MENU_IDS.fields, label: $localize`:@@shell.menu.view.fields:Fields` },
    ...[...core, ...extraCore].map(viewItem),
    cmd('timeline.new'),
    SEP,
    ...submenus,
    SEP,
    { kind: 'item', id: MENU_IDS.otherWindows, label: $localize`:@@shell.menu.view.otherWindows:Other windows…` },
    SEP,
    sub('view.theme', $localize`:@@shell.menu.view.theme:Theme`, [
      cmd('view.theme.system', $localize`:@@shell.menu.view.theme.system:System`, state.theme === 'system'),
      cmd('view.theme.dark', $localize`:@@shell.menu.view.theme.dark:Dark`, state.theme === 'dark'),
      cmd('view.theme.light', $localize`:@@shell.menu.view.theme.light:Light`, state.theme === 'light'),
    ]),
    sub('view.sourceColors', $localize`:@@shell.menu.view.sourceColors:Source colours`, [
      cmd('view.sourceColors.source', $localize`:@@shell.menu.view.sourceColors.source:By file`, (state.sourceColors ?? 'source') === 'source'),
      cmd('view.sourceColors.instance', $localize`:@@shell.menu.view.sourceColors.instance:By instance`, state.sourceColors === 'instance'),
      cmd('view.sourceColors.off', $localize`:@@shell.menu.view.sourceColors.off:Off`, state.sourceColors === 'off'),
    ]),
  ]);
}

/** The plugin manager, then the views of each plugin under the plugin's name (only labelled when several). */
function pluginItems(views: readonly MenuView[]): MenuNode[] {
  const manager = views.filter((v) => v.category === 'Plugins' && !v.plugin);
  const byPlugin = new Map<string, MenuView[]>();
  for (const v of views) {
    if (manager.includes(v)) continue;
    const name = v.plugin ?? $localize`:@@shell.menu.view.otherPlugins:Other plugins`;
    byPlugin.set(name, [...(byPlugin.get(name) ?? []), v]);
  }
  const groups: MenuNode[] = [...byPlugin.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, vs]) => ({ kind: 'group', label, items: vs.map(viewItem) }));
  return tidy([...manager.map(viewItem), SEP, ...groups]);
}

/** Layouts, tool window and document actions, then the open windows and pop-outs. */
function windowMenu(state: MenuState, cmd: Cmd, sub: Sub): MenuNode[] {
  const windows: MenuItemNode[] = state.windows.map((w) => ({ kind: 'item', id: MENU_IDS.window + w.label, label: w.title || w.label, checked: w.current }));
  const popouts: MenuItemNode[] = state.popouts.map((p) => ({ kind: 'item', id: MENU_IDS.popout + p.id, label: p.title }));
  return tidy([
    sub('window.layouts', $localize`:@@shell.menu.window.layouts:Layouts`, [
      cmd('window.layout.save'),
      SEP,
      ...[1, 2, 3, 4].map((n) => cmd(`window.layout.apply${n}`)),
      SEP,
      cmd('window.layout.manage'),
      cmd('window.layout.reset'),
    ]),
    SEP,
    cmd('window.autoHideAll'),
    cmd('window.pinAll'),
    cmd('window.closeAllDocuments'),
    SEP,
    cmd('window.popOut'),
    cmd('window.fullScreen'),
    SEP,
    windows.length > 1 || popouts.length ? { kind: 'group', label: $localize`:Menu group@@shell.menu.window.windows:Windows`, items: windows } : null,
    popouts.length ? { kind: 'group', label: $localize`:@@shell.menu.window.popoutPanels:Pop-out panels`, items: popouts } : null,
  ]);
}

/** Every item in a menu tree, depth first. */
export function menuItems(nodes: readonly MenuNode[]): MenuItemNode[] {
  return nodes.flatMap((n) => (n.kind === 'item' ? [n] : n.kind === 'submenu' || n.kind === 'group' ? menuItems(n.items) : []));
}

/** Number of entries at the top level of a menu (a group counts its items and its label). */
export function topLevelSize(nodes: readonly MenuNode[]): number {
  return nodes.reduce((n, node) => n + (node.kind === 'group' ? node.items.length + 1 : 1), 0);
}
