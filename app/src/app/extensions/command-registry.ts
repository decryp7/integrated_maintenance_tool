import { computed, inject, Injectable, InjectionToken, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Logger } from '../core/logger';
import { ActivityLog } from '../core/activity/activity-log';
import { Trigger } from '../core/activity/types';
import { fuzzyFilter } from './fuzzy';
import { matchesKeybinding } from './keybinding';
import { applyOverrides } from './keybinding-overrides';
import { KeybindingOverridesStore } from './keybinding-overrides.store';
import { CommandContext, flattenContributions, DEBRIEF_COMMANDS, DebriefCommand } from './tokens';
import { WorkspaceCommandContext } from './command-context';

/** The context handed to commands. Replaceable in tests. */
export const COMMAND_CONTEXT = new InjectionToken<CommandContext>('COMMAND_CONTEXT', {
  providedIn: 'root',
  factory: () => inject(WorkspaceCommandContext),
});

/** All registered commands (built in and from plugins), their availability and execution. */
@Injectable({ providedIn: 'root' })
export class CommandRegistry {
  private readonly contributed = flattenContributions<DebriefCommand>(inject(DEBRIEF_COMMANDS, { optional: true }));
  private readonly store = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly context = inject(COMMAND_CONTEXT);
  private readonly activity = inject(ActivityLog);
  private readonly overrides = inject(KeybindingOverridesStore);

  /** Commands once each, with the user's key bindings applied (a cleared binding has no `keybinding`). */
  private readonly effective = computed(() => {
    const seen = new Set<string>();
    return applyOverrides(
      this.contributed.filter((c) => !seen.has(c.id) && !!seen.add(c.id)),
      this.overrides.overrides(),
    );
  });

  /** While true (the shortcut editor is recording a key) key presses run no command. */
  readonly keysSuspended = signal(false);

  /** Commands whose `when` currently holds. Re-evaluated when the workspace changes. */
  readonly available = computed(() => {
    this.store.summary();
    return this.all().filter((c) => this.isAvailable(c));
  });

  /** Every command with its effective key binding. */
  all(): readonly DebriefCommand[] {
    return this.effective();
  }

  /** The binding the command declares itself, ignoring the user's overrides. */
  declaredKeybinding(id: string): string | undefined {
    return this.contributed.find((c) => c.id === id)?.keybinding;
  }

  find(id: string): DebriefCommand | undefined {
    return this.all().find((c) => c.id === id);
  }

  search(query: string): DebriefCommand[] {
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
    if (this.keysSuspended()) return false;
    const command = this.available().find((c) => c.keybinding && matchesKeybinding(c.keybinding, e));
    if (!command) return false;
    e.preventDefault();
    void this.run(command.id, 'shortcut');
    return true;
  }

  private isAvailable(c: DebriefCommand): boolean {
    try {
      return c.when ? c.when(this.context) : true;
    } catch {
      return false;
    }
  }
}
