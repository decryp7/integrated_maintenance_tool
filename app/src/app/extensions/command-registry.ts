import { computed, inject, Injectable, InjectionToken } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Logger } from '../core/logger';
import { ActivityLog } from '../core/activity/activity-log';
import { Trigger } from '../core/activity/types';
import { fuzzyFilter } from './fuzzy';
import { matchesKeybinding } from './keybinding';
import { CommandContext, flattenContributions, IMT_COMMANDS, ImtCommand } from './tokens';
import { WorkspaceCommandContext } from './command-context';

/** The context handed to commands. Replaceable in tests. */
export const COMMAND_CONTEXT = new InjectionToken<CommandContext>('COMMAND_CONTEXT', {
  providedIn: 'root',
  factory: () => inject(WorkspaceCommandContext),
});

/** All registered commands (built in and from plugins), their availability and execution. */
@Injectable({ providedIn: 'root' })
export class CommandRegistry {
  private readonly contributed = flattenContributions<ImtCommand>(inject(IMT_COMMANDS, { optional: true }));
  private readonly store = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly context = inject(COMMAND_CONTEXT);
  private readonly activity = inject(ActivityLog);

  /** Commands whose `when` currently holds. Re-evaluated when the workspace changes. */
  readonly available = computed(() => {
    this.store.summary();
    return this.all().filter((c) => this.isAvailable(c));
  });

  all(): ImtCommand[] {
    const seen = new Set<string>();
    return this.contributed.filter((c) => !seen.has(c.id) && !!seen.add(c.id));
  }

  find(id: string): ImtCommand | undefined {
    return this.all().find((c) => c.id === id);
  }

  search(query: string): ImtCommand[] {
    return fuzzyFilter(this.available(), query, (c) => (c.category ? `${c.category}: ${c.title}` : c.title));
  }

  /**
   * Runs a command by id. Failures are shown to the user, never thrown to the caller. Every run is one entry in
   * the activity log (`User <id> via <trigger>`); without a `trigger` it is inferred from the latest user input
   * (menu, palette, shortcut, context menu, button, native menu event), see `core/activity/trigger.ts`.
   */
  async run(id: string, trigger: Trigger | 'auto' = 'auto'): Promise<boolean> {
    const command = this.find(id);
    if (!command || !this.isAvailable(command)) return false;
    try {
      await this.activity.track(id, trigger, { Title: command.title, Category: command.category }, () => command.run(this.context));
      return true;
    } catch (e) {
      this.logger.warn('ui.command', 'Command failed', { Command: id });
      this.store.setError($localize`:@@extensions.commandRegistry.failed:${command.title}:title: failed: ${e instanceof Error ? e.message : String(e)}:detail:`);
      return false;
    }
  }

  /** Runs the command bound to the key combination, if any. Returns true when one ran. */
  runForKey(e: KeyboardEvent): boolean {
    const command = this.available().find((c) => c.keybinding && matchesKeybinding(c.keybinding, e));
    if (!command) return false;
    e.preventDefault();
    void this.run(command.id, 'shortcut');
    return true;
  }

  private isAvailable(c: ImtCommand): boolean {
    try {
      return c.when ? c.when(this.context) : true;
    } catch {
      return false;
    }
  }
}
