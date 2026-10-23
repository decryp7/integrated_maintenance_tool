import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal } from '@angular/core';
import { PluginMemoryStore } from '../settings/performance/plugin-memory.store';
import { MAX_PLUGIN_MB, MIN_PLUGIN_MB } from '../settings/performance/performance.types';

/**
 * A plugin's memory cap in the detail panel (ADR 0031): its own cap over the default (applied from the plugin's next
 * start), saved when the field loses focus or on Enter. The engine validates the cap.
 */
@Component({
  selector: 'imt-plugin-memory',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (row(); as r) {
      <div class="mem" data-testid="plugin-memory">
        <label class="cap">
          <span class="lbl" i18n="@@pluginsPanel.memory.own">Memory cap</span>
          <span class="field">
            <input class="input" type="number" [min]="min" [max]="max" step="64" [value]="draft() ?? r.capMb" (input)="draft.set(num($event))" (blur)="save()" (keydown.enter)="save()" [attr.aria-label]="label()" data-testid="plugin-memory-input" />
            <span class="unit">MB</span>
          </span>
        </label>
        <span class="faint note">
          @if (r.peakMb !== null) {
            <ng-container i18n="@@pluginsPanel.memory.peak">Peak {{ r.peakMb }} MB of a {{ r.capMb }} MB cap.</ng-container>
          } @else {
            <ng-container i18n="@@pluginsPanel.memory.cap">The cap is {{ r.capMb }} MB.</ng-container>
          }
          <ng-container i18n="@@pluginsPanel.memory.applies">A new cap applies from the plugin's next start.</ng-container>
        </span>
        @if (r.overrideMb !== null) {
          <button type="button" class="btn ghost link" (click)="clear()" data-testid="plugin-memory-clear" i18n="@@pluginsPanel.memory.clear">Use the default</button>
        }
        @if (error(); as e) {
          <span class="err" role="alert">{{ e }}</span>
        }
      </div>
    }
  `,
  styles: `
    .mem { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); align-items: flex-start; }
    .cap { display: flex; flex-direction: column; gap: 4px; }
    .lbl { color: var(--muted); }
    .field { display: inline-flex; align-items: center; gap: 6px; }
    input { width: 96px; font-variant-numeric: tabular-nums; }
    .unit { color: var(--muted); }
    .note { line-height: 1.4; }
    .link { padding-inline: 0; }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class PluginMemory {
  readonly pluginId = input.required<string>();
  readonly pluginName = input<string>('');
  private readonly store = inject(PluginMemoryStore);
  protected readonly min = MIN_PLUGIN_MB;
  protected readonly max = MAX_PLUGIN_MB;
  protected readonly row = computed(() => this.store.processes().find((p) => p.process.kind === 'plugin' && p.process.pluginId === this.pluginId()));
  protected readonly draft = signal<number | null>(null);
  protected readonly error = signal<string | null>(null);
  protected readonly label = computed(() => $localize`:@@pluginsPanel.memory.aria:Memory cap of ${this.pluginName() || this.pluginId()}:name: in MB`);

  constructor() {
    // The plugin's own cap, or the value a memory message suggested ("Raise the cap…").
    effect(() => {
      const asked = this.store.requested();
      const r = this.row();
      this.draft.set(asked?.pluginId === this.pluginId() ? asked.mb : (r?.overrideMb ?? null));
    });
  }

  protected num(e: Event): number | null {
    const v = (e.target as HTMLInputElement).valueAsNumber;
    return Number.isFinite(v) ? v : null;
  }

  /** Saves a changed cap (on blur or Enter); an unchanged or empty field does nothing. */
  protected async save(): Promise<void> {
    const mb = this.draft();
    if (mb === null || mb === this.row()?.overrideMb) return;
    this.error.set(await this.store.setCap(this.pluginId(), mb));
  }

  protected async clear(): Promise<void> {
    this.error.set(await this.store.setCap(this.pluginId(), null));
  }
}
