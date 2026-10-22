import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal } from '@angular/core';
import { PluginMemoryStore } from '../settings/performance/plugin-memory.store';
import { MAX_PLUGIN_MB, MIN_PLUGIN_MB } from '../settings/performance/performance.types';

/**
 * A plugin's memory in the Plugins panel (ADR 0031): its cap, the peak the memory watcher last reported, and its own
 * cap over the default (applied from the plugin's next start). The engine validates the cap.
 */
@Component({
  selector: 'imt-plugin-memory',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (row(); as r) {
      <div class="mem" data-testid="plugin-memory">
        <span class="faint">
          @if (r.peakMb !== null) {
            <ng-container i18n="@@pluginsPanel.memory.peak">Memory: peak {{ r.peakMb }} MB of a {{ r.capMb }} MB cap</ng-container>
          } @else {
            <ng-container i18n="@@pluginsPanel.memory.cap">Memory cap: {{ r.capMb }} MB</ng-container>
          }
          @if (r.state === 'stopped') {
            <strong class="warn" i18n="@@pluginsPanel.memory.stopped">· stopped over its cap</strong>
          } @else if (r.state === 'warned') {
            <span class="warn" i18n="@@pluginsPanel.memory.warned">· near its cap</span>
          }
        </span>
        <label class="cap">
          <span i18n="@@pluginsPanel.memory.own">Own cap</span>
          <input type="number" [min]="min" [max]="max" step="64" [value]="draft() ?? ''" (input)="draft.set(num($event))" [attr.aria-label]="label()" data-testid="plugin-memory-input" />
          <span class="faint">MB</span>
        </label>
        <button type="button" class="btn" [disabled]="draft() === null || draft() === r.overrideMb" (click)="save()" data-testid="plugin-memory-set" i18n="@@pluginsPanel.memory.set">Set</button>
        @if (r.overrideMb !== null) {
          <button type="button" class="btn ghost" (click)="clear()" data-testid="plugin-memory-clear" i18n="@@pluginsPanel.memory.clear">Use the default</button>
        }
        @if (error(); as e) {
          <span class="err" role="alert">{{ e }}</span>
        }
      </div>
    }
  `,
  styles: `
    .mem { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: var(--fs-sm); }
    .cap { display: inline-flex; gap: 4px; align-items: center; }
    input { width: 80px; }
    .warn { color: var(--lvl-warn); }
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

  protected async save(): Promise<void> {
    this.error.set(await this.store.setCap(this.pluginId(), this.draft()));
  }

  protected async clear(): Promise<void> {
    this.error.set(await this.store.setCap(this.pluginId(), null));
  }
}
