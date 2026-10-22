import { computed, inject, Injectable } from '@angular/core';
import { Logger } from '../core/logger';
import { DockService } from '../dock/dock.service';
import { COMMAND_CONTEXT, CommandRegistry } from './command-registry';
import { CommandContext, flattenContributions, IMT_ACTIVITY_ITEMS, ImtActivityItem, ImtActivityMenuItem, ImtActivityTarget, VIEW_OPEN_PREFIX, ViewCategory } from './tokens';
import { ViewRegistry } from './view-registry';
import { WelcomeService } from '../welcome/welcome';

const LOG = 'ui.activity';

/** `when` of activity items that only make sense with data open (the bar stays short on an empty window). */
export function whenDataOpen(ctx: CommandContext): boolean {
  return (ctx.summary?.sources.length ?? 0) > 0;
}

/** True for `{ view }` or `{ command }` with a non-empty id. */
function isTarget(run: unknown): run is ImtActivityTarget {
  if (!run || typeof run !== 'object') return false;
  const r = run as Record<string, unknown>;
  return (typeof r['view'] === 'string' && !!r['view']) || (typeof r['command'] === 'string' && !!r['command']);
}

function isMenu(run: ImtActivityItem['run']): run is { menu: () => readonly ImtActivityMenuItem[] } {
  return typeof (run as { menu?: unknown }).menu === 'function';
}

/** Why a contribution cannot be used, or null when it is well-formed. Plugin input is untrusted. */
export function activityItemProblem(item: unknown): string | null {
  if (!item || typeof item !== 'object') return 'not an object';
  const i = item as Partial<ImtActivityItem>;
  if (typeof i.id !== 'string' || !i.id) return 'missing id';
  if (typeof i.label !== 'string' || !i.label) return 'missing label';
  if (typeof i.icon !== 'string') return 'missing icon';
  if (typeof i.order !== 'number' || !Number.isFinite(i.order)) return 'order is not a number';
  if (i.section !== undefined && i.section !== 'top' && i.section !== 'bottom') return 'unknown section';
  if (!i.run || (!isTarget(i.run) && !isMenu(i.run))) return 'run is not a view, a command or a menu';
  return null;
}

/**
 * The activity bar's buttons, contributed through `IMT_ACTIVITY_ITEMS` (built-in features and plugins alike):
 * validated, de-duplicated by id (first wins) and sorted by `order`. Broken contributions are logged and left out,
 * and every contributed function is evaluated defensively, so one bad item never breaks the bar.
 */
@Injectable({ providedIn: 'root' })
export class ActivityItems {
  private readonly logger = inject(Logger);
  private readonly ctx = inject<CommandContext>(COMMAND_CONTEXT);
  private readonly commands = inject(CommandRegistry);
  private readonly views = inject(ViewRegistry);
  private readonly dock = inject(DockService);
  private readonly welcome = inject(WelcomeService);
  /** `<item id>:<what>` already reported, so a function that keeps throwing logs once. */
  private readonly reported = new Set<string>();

  /** Every usable contribution, in order. */
  readonly all: readonly ImtActivityItem[] = this.collect(flattenContributions<ImtActivityItem>(inject(IMT_ACTIVITY_ITEMS, { optional: true })));

  /** The items shown right now (their `when` holds, the view they show is offered, their fly-out is not empty). */
  readonly visible = computed(() => {
    const offered = new Set(this.views.available().map((v) => v.id));
    return this.all.filter((item) => {
      if ('view' in item.run && !offered.has(item.run.view)) return false;
      if (item.when && !this.safe(item, 'when', () => item.when?.(this.ctx) === true, false)) return false;
      // A fly-out with nothing to offer (e.g. no tool listed on this platform) is left out.
      return !isMenu(item.run) || this.menu(item).length > 0;
    });
  });

  readonly top = computed(() => this.visible().filter((i) => i.section !== 'bottom'));
  readonly bottom = computed(() => this.visible().filter((i) => i.section === 'bottom'));

  /** Highlighted: its own `active`, else the view it shows is in front. */
  isActive(item: ImtActivityItem): boolean {
    if (item.active) return this.safe(item, 'active', () => item.active?.(this.ctx) === true, false);
    return 'view' in item.run ? this.dock.isVisible(item.run.view) : false;
  }

  /**
   * A fly-out item: `shown` while one of its views is in front, `open` while some are open only behind others or
   * auto-hidden, else null. Entries that run a command say nothing about views.
   */
  menuState(item: ImtActivityItem): 'shown' | 'open' | null {
    const views = this.menu(item).flatMap((e) => ('view' in e.run ? [e.run.view] : []));
    if (views.some((v) => this.dock.isVisible(v))) return 'shown';
    return views.some((v) => this.dock.isOpen(v)) ? 'open' : null;
  }

  /** The item's view is open but not in front (a tab behind another, or auto-hidden): the bar marks it with a dot. */
  isOpenElsewhere(item: ImtActivityItem): boolean {
    return !item.active && 'view' in item.run && this.dock.isOpen(item.run.view) && !this.dock.isVisible(item.run.view);
  }

  /** The badge text, or null for none. */
  badge(item: ImtActivityItem): string | null {
    if (!item.badge) return null;
    const value = this.safe(item, 'badge', () => item.badge?.(this.ctx) ?? null, null);
    if (value === null || value === '') return null;
    return typeof value === 'number' ? (value > 99 ? '99+' : String(value)) : String(value).slice(0, 4);
  }

  pulse(item: ImtActivityItem): boolean {
    return item.pulse ? this.safe(item, 'pulse', () => item.pulse?.(this.ctx) === true, false) : false;
  }

  /** Why the item is inert right now, or null when it can be used. */
  disabledReason(item: ImtActivityItem): string | null {
    if (!item.disabled) return null;
    const reason = this.safe(item, 'disabled', () => item.disabled?.(this.ctx) ?? null, null);
    return typeof reason === 'string' && reason ? reason : null;
  }

  /** The key binding (command syntax or display text): its own, else its command's, else its view's "Open <view>" command's. */
  keybinding(item: ImtActivityItem | ImtActivityMenuItem): string | null {
    if (item.keybinding) return item.keybinding;
    const run = item.run;
    if (isTarget(run)) return this.commands.find('command' in run ? run.command : `${VIEW_OPEN_PREFIX}${run.view}`)?.keybinding ?? null;
    return null;
  }

  /** The fly-out entries of a menu item: well-formed, unique, and only those that can run now. */
  menu(item: ImtActivityItem): ImtActivityMenuItem[] {
    if (!isMenu(item.run)) return [];
    const run = item.run;
    const entries = this.safe(item, 'menu', () => [...run.menu()], []);
    const offered = new Set(this.views.available().map((v) => v.id));
    const runnable = new Set(this.commands.available().map((c) => c.id));
    const seen = new Set<string>();
    return entries.filter((e) => {
      if (!e || typeof e.id !== 'string' || typeof e.label !== 'string' || !isTarget(e.run) || seen.has(e.id)) return false;
      seen.add(e.id);
      return 'view' in e.run ? offered.has(e.run.view) : runnable.has(e.run.command);
    });
  }

  hasMenu(item: ImtActivityItem): boolean {
    return isMenu(item.run);
  }

  /** Shows the view or runs the command. Unavailable commands are ignored by the registry. */
  run(target: ImtActivityTarget): void {
    if ('view' in target) {
      // The welcome screen covers the dock while nothing is open: the view asked for must be visible.
      this.welcome.stepAside();
      this.dock.show(target.view);
    }
    else void this.commands.run(target.command, 'button');
  }

  private collect(items: readonly ImtActivityItem[]): ImtActivityItem[] {
    const seen = new Set<string>();
    const out: ImtActivityItem[] = [];
    for (const item of items) {
      const problem = activityItemProblem(item);
      if (problem) {
        this.logger.warn(LOG, 'Activity item skipped', { Reason: problem });
        continue;
      }
      if (seen.has(item.id)) {
        this.logger.warn(LOG, 'Duplicate activity item skipped', { Item: item.id });
        continue;
      }
      seen.add(item.id);
      out.push(item);
    }
    // Stable: equal orders keep their registration order.
    return out.map((item, i) => ({ item, i })).sort((a, b) => a.item.order - b.item.order || a.i - b.i).map((x) => x.item);
  }

  private safe<T>(item: ImtActivityItem, what: string, fn: () => T, fallback: T): T {
    try {
      return fn();
    } catch {
      const key = `${item.id}:${what}`;
      if (!this.reported.has(key)) {
        this.reported.add(key);
        this.logger.warn(LOG, 'Activity item failed', { Item: item.id, Part: what });
      }
      return fallback;
    }
  }
}

/**
 * A fly-out listing the offered views of one View-menu category (with a check mark on the open ones), then the given
 * commands. `first` lists view ids to put at the top, in that order. Call it in an injection context (a provider's
 * `useFactory`).
 */
export function viewCategoryMenu(category: ViewCategory, options: { first?: readonly string[]; commands?: readonly string[] } = {}): () => ImtActivityMenuItem[] {
  const views = inject(ViewRegistry);
  const dock = inject(DockService);
  const commands = inject(CommandRegistry);
  return () => {
    const listed = views.available().filter((v) => (v.category ?? 'Plugins') === category && v.menu !== 'palette');
    const first = (options.first ?? []).map((id) => listed.find((v) => v.id === id)).filter((v) => !!v);
    const ordered = [...first, ...listed.filter((v) => !first.includes(v))];
    return [
      ...ordered.map((v): ImtActivityMenuItem => ({ id: `view:${v.id}`, label: v.title, icon: v.icon, checked: dock.isOpen(v.id), run: { view: v.id } })),
      ...(options.commands ?? []).flatMap((id): ImtActivityMenuItem[] => {
        const c = commands.find(id);
        return c ? [{ id: `command:${id}`, label: c.title, run: { command: id } }] : [];
      }),
    ];
  };
}
