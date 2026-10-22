import { InjectionToken, InputSignal, Type } from '@angular/core';
import { FieldFilter, Query, RecordView, WorkspaceSummary } from '../core/api.types';

/** Where a new view opens when the user has not placed it yet. */
export type ViewPosition = 'center' | 'right' | 'bottom';

/**
 * A view's home area in the window (where it opens unless the user moved it):
 * - `document`: the centre, beside (never over) the timeline and the events grid;
 * - `analysis`: the one analysis panel below the events grid (results you read next to the events);
 * - `side`: a tool window on the right (inspector, assistant, notes);
 * - `status`: a bottom tool window that slides out over the dock and hides again (jobs, notifications, problems).
 */
export type ViewArea = 'document' | 'analysis' | 'side' | 'status';

/**
 * Where a view is listed in the View menu: `core` views at the top level, the others in a submenu of that name.
 * Views without a category are plugin views and land in View ▸ Plugins (grouped by `plugin`).
 */
export type ViewCategory = 'core' | 'Analysis' | 'Live' | 'Profiling' | 'Tools' | 'Sharing' | 'Diagnostics' | 'Plugins' | 'AI';

/** A dockable panel contributed by a plugin (or built in). */
export interface ImtView {
  /** Unique, stable id (also the dock panel id and the key in saved layouts). */
  id: string;
  title: string;
  /** Name of an `<imt-icon>` icon. */
  icon: string;
  /** Lazily loads the standalone component rendered inside the panel. */
  component: () => Promise<Type<unknown>>;
  defaultPosition?: ViewPosition;
  /** Its home area (see `ViewArea`); derived from the kind and `defaultPosition` when left out (`viewArea`). */
  area?: ViewArea;
  /** Its place in the View menu (see `ViewCategory`). Defaults to the Plugins submenu. */
  category?: ViewCategory;
  /** Name of the plugin that contributes the view; groups plugin views in View ▸ Plugins. */
  plugin?: string;
  /**
   * `'palette'`: a granular view reached from inside another view; it is left out of the View menu but still offered
   * by the palette and View ▸ Other windows…. Defaults to listed.
   */
  menu?: 'palette';
  /** The view is offered (View menu, palette) only while this returns true. Defaults to always. */
  when?: (summary: WorkspaceSummary) => boolean;
  /**
   * Opens the view (in the background) when `when` starts to hold, e.g. a viewer for the kind of file just opened.
   * Without it, a view with `when` is only offered and opens on demand. It is always closed when `when` stops holding.
   * `full`: only in the Full layout (View › Layout); in the Focused layout it is offered and opens on demand.
   */
  autoOpen?: boolean | 'full';
  /**
   * Like `when`, for a feature that may be absent (its engine plugin is disabled, or the app runs in safe mode): the
   * view is offered only while this returns true, but it is never opened or closed automatically. Defaults to always.
   */
  enabled?: () => boolean;
  /**
   * The view can be open several times: panels `<id>#<key>` render it too, and the component tells them apart by
   * `DOCK_PANEL_ID`. Defaults to a single panel.
   */
  instances?: boolean;
}

/** The only handle plugins get on the app; deliberately small (see `ImtEvents` for reading state). */
export interface CommandContext {
  /** Opens files or folders in the workspace. */
  openPaths(paths: string[]): Promise<void>;
  /** Applies a partial query (search text, levels, range, ...). */
  setQuery(patch: Partial<Query>): void;
  /** Adds a field filter pill. */
  addFilter(filter: FieldFilter): void;
  /** Selects a record (shown in the inspector). */
  selectRecord(record: RecordView | null): void;
  /**
   * Reveals a record by id: loads the grid page containing it, scrolls to the row and selects it.
   * Resolves once the grid has done so or reported that the record is not in the current view.
   */
  jumpTo(id: number): Promise<void>;
  /** Opens (if needed) and focuses a view by id. */
  showView(id: string): void;
  /**
   * Offers the window arrangement for what the user just opened (`profiling` after a performance capture): switches
   * to that perspective once per window, with an Undo in the notice. Unknown ids and repeats do nothing.
   */
  suggestPerspective?(id: string): void;
  /** Reports a problem to the user. */
  notifyError(message: string): void;
  /** The current workspace summary (null before the first load). */
  readonly summary: WorkspaceSummary | null;
}

/** An action in the command palette and menus, optionally bound to a key. */
export interface ImtCommand {
  id: string;
  title: string;
  /** Groups commands in the palette; also selects the menu in the title bar for built-ins. */
  category?: string;
  /** e.g. "Mod+Shift+P" (Mod = Ctrl, or Cmd on macOS). Shown as a hint in the palette. */
  keybinding?: string;
  /** The command is listed and runnable only while this returns true. */
  when?: (ctx: CommandContext) => boolean;
  /**
   * The view this command only shows. The palette lists one command per view: this one, and not the generated
   * "Open <view>" (`view.open.<id>`) as well.
   */
  view?: string;
  run: (ctx: CommandContext) => void | Promise<void>;
}

/** Id prefix of the generated "Open <view>" commands, one per registered view. */
export const VIEW_OPEN_PREFIX = 'view.open.';

/** A block rendered in the inspector below the properties. */
export interface ImtInspectorSection {
  id: string;
  title: string;
  /** Ascending; built-in sections use multiples of 100. */
  order: number;
  /** Standalone component with a required `record` signal input. */
  component: Type<{ record: InputSignal<RecordView> }>;
  when?: (record: RecordView) => boolean;
}

/** Formats a field's value for display in the grid and inspector. */
export interface ImtFieldFormatter {
  /** Exact field name, or a pattern tested against it. */
  field: string | RegExp;
  format: (value: unknown, record: RecordView) => string;
}

/** What an activity-bar button or fly-out entry does: show a view, or run a command. */
export type ImtActivityTarget = { view: string } | { command: string };

/** One entry of an activity-bar fly-out menu. */
export interface ImtActivityMenuItem {
  /** Unique within the menu. */
  id: string;
  label: string;
  /** Name of an `<imt-icon>` icon. */
  icon?: string;
  /** Shows a check mark (e.g. the view is open). */
  checked?: boolean;
  /** Key binding in command syntax, shown as a hint. */
  keybinding?: string;
  run: ImtActivityTarget;
}

/**
 * A button in the left activity bar (built in or from a plugin). `run` shows a view, runs a command, or opens a
 * fly-out menu whose entries are built on every opening. Every function is evaluated defensively: one that throws
 * hides the button (`when`) or drops just that extra (`badge`, `active`, `pulse`, `disabled`, `menu`), never the bar.
 */
export interface ImtActivityItem {
  /** Unique, stable id (also `data-activity-id` on the button). */
  id: string;
  /** Accessible name and tooltip. */
  label: string;
  /** Name of an `<imt-icon>` icon. */
  icon: string;
  /** Ascending, top to bottom within its section; built-in items use multiples of 100. Higher orders collapse into More first. */
  order: number;
  /** `bottom`: pinned at the bottom of the bar (Settings). Defaults to `top`. */
  section?: 'top' | 'bottom';
  /** Shown only while this returns true. Items that show a view are also hidden while the view is not offered. */
  when?: (ctx: CommandContext) => boolean;
  /** A short count or label drawn on the icon (null or '' draws none). */
  badge?: (ctx: CommandContext) => string | number | null;
  /** Highlighted while true. Defaults to: the item's view is open (items that show a view), else never. */
  active?: (ctx: CommandContext) => boolean;
  /** A toggle button: `aria-pressed` follows `active`. */
  toggle?: boolean;
  /** A small pulsing dot while true (e.g. following live). */
  pulse?: (ctx: CommandContext) => boolean;
  /** Shown but inert while this returns a reason, which becomes the tooltip. */
  disabled?: (ctx: CommandContext) => string | null;
  /** Key binding in command syntax (e.g. "Mod+B"), or a key as shown (e.g. "/"). Defaults to the run command's binding. */
  keybinding?: string;
  run: ImtActivityTarget | { menu: () => readonly ImtActivityMenuItem[] };
}

/**
 * Multi-provider tokens. Each provider supplies one batch (an array) of contributions, so
 * `provideImtPlugin` registers a whole plugin with one provider per kind. Readers flatten the
 * batches with `flattenContributions`.
 */
export const IMT_VIEWS = new InjectionToken<readonly ImtView[]>('IMT_VIEWS');
export const IMT_COMMANDS = new InjectionToken<readonly ImtCommand[]>('IMT_COMMANDS');
export const IMT_INSPECTOR_SECTIONS = new InjectionToken<readonly ImtInspectorSection[]>('IMT_INSPECTOR_SECTIONS');
export const IMT_FIELD_FORMATTERS = new InjectionToken<readonly ImtFieldFormatter[]>('IMT_FIELD_FORMATTERS');
export const IMT_ACTIVITY_ITEMS = new InjectionToken<readonly ImtActivityItem[]>('IMT_ACTIVITY_ITEMS');

/** Flattens what a multi-provider token injects (batches, or single items) into one list. */
export function flattenContributions<T>(injected: readonly (T | readonly T[])[] | null | undefined): T[] {
  return ((injected ?? []) as (T | T[])[]).flat() as T[];
}
