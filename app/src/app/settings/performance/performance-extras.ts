import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { EngineSettingsStore } from '../engine-settings.store';
import { SettingValue } from '../settings.types';
import { PerformanceBackend } from './performance-backend';
import { Environment, PERFORMANCE_SECTION, Preset, Proposal } from './performance.types';

/**
 * Above the Performance fields: Suggest settings (values the engine computes for this computer) and the presets
 * (Standard, Low memory), each shown as a before/after table to apply. Applying goes through the section's normal
 * Apply, so the engine validates the values like any edit.
 */
@Component({
  selector: 'imt-performance-extras',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar">
      <button type="button" class="btn primary" [disabled]="busy()" (click)="suggest()" data-testid="perf-suggest" i18n="@@performance.suggest">Suggest settings</button>
      <span class="faint" i18n="@@performance.presets">Presets:</span>
      <button type="button" class="btn" [disabled]="busy()" (click)="preset('standard')" data-testid="perf-preset-standard" i18n="@@performance.preset.standard">Standard</button>
      <button type="button" class="btn" [disabled]="busy()" (click)="preset('lowMemory')" data-testid="perf-preset-low" i18n="@@performance.preset.lowMemory">Low memory</button>
    </div>
    @if (error(); as e) {
      <p class="error" role="alert">{{ e }}</p>
    }
    @if (proposal(); as p) {
      <div class="proposal" data-testid="perf-proposal">
        @if (p.environment; as env) {
          <p class="faint" data-testid="perf-env">{{ envText(env) }}</p>
        }
        @if (p.rows.length) {
          <table>
            <thead>
              <tr>
                <th scope="col" i18n="@@performance.table.setting">Setting</th>
                <th scope="col" i18n="@@performance.table.now">Now</th>
                <th scope="col" i18n="@@performance.table.suggested">Suggested</th>
              </tr>
            </thead>
            <tbody>
              @for (r of p.rows; track r.key) {
                <tr>
                  <th scope="row">{{ r.label }}</th>
                  <td>{{ show(r.current, r.unit) }}</td>
                  <td>{{ show(r.suggested, r.unit) }}</td>
                </tr>
              }
            </tbody>
          </table>
          <div class="bar">
            <button type="button" class="btn primary" [disabled]="busy()" (click)="apply(p)" data-testid="perf-apply" i18n="@@performance.apply">Apply these values</button>
            <button type="button" class="btn ghost" (click)="proposal.set(null)" i18n="@@performance.dismiss">Keep the current values</button>
          </div>
        } @else {
          <p class="faint" data-testid="perf-nothing" i18n="@@performance.nothing">The current values are already these.</p>
        }
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
    .bar { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .proposal { border: 1px solid var(--border-2); border-radius: 8px; padding: 8px 12px; background: var(--bg-2); display: flex; flex-direction: column; gap: 8px; }
    table { border-collapse: collapse; font-size: var(--fs-md); }
    th, td { text-align: left; padding: 2px 12px 2px 0; }
    thead th { color: var(--text-2); font-weight: 500; }
    tbody th { font-weight: normal; }
    .error { color: var(--danger-text, var(--lvl-error)); margin: 0; }
  `,
})
export class PerformanceExtras {
  private readonly backend = inject(PerformanceBackend);
  private readonly settings = inject(EngineSettingsStore);
  protected readonly proposal = signal<Proposal | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);

  protected gb(mb: number): string {
    return (mb / 1024).toFixed(mb >= 10 * 1024 ? 0 : 1);
  }

  protected envText(env: Environment): string {
    const base = $localize`:@@performance.environment:For this computer: ${this.gb(env.totalMemoryMb)}:total: GB of memory (${this.gb(env.freeMemoryMb)}:free: GB free), ${env.cores}:cores: cores`;
    return env.aiModelMb ? $localize`:@@performance.environmentAi:${base}:base:, minus an AI model of ${this.gb(env.aiModelMb)}:model: GB` : base;
  }

  protected show(v: SettingValue, unit: string | null): string {
    if (typeof v === 'boolean') return v ? $localize`:@@performance.on:On` : $localize`:@@performance.off:Off`;
    if (v === '') return $localize`:@@performance.empty:(none)`;
    return unit ? `${v} ${unit}` : String(v);
  }

  private async load(f: () => Promise<Proposal>): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      this.proposal.set(await f());
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected suggest(): Promise<void> {
    return this.load(() => this.backend.suggest());
  }

  protected preset(p: Preset): Promise<void> {
    return this.load(() => this.backend.preset(p));
  }

  /** Puts the proposal's values into the section and applies it (the engine validates them). */
  protected async apply(p: Proposal): Promise<void> {
    this.busy.set(true);
    try {
      for (const r of p.rows) this.settings.edit(PERFORMANCE_SECTION, r.key, p.values[r.key] ?? r.suggested);
      if (await this.settings.apply()) this.proposal.set(null);
    } finally {
      this.busy.set(false);
    }
  }
}
