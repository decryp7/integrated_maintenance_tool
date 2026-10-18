import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { AiBackend, toAiError } from '../ai-backend';
import { formatBytes } from '../ai-format';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';

/**
 * Settings → AI, above the fields: set-up and model buttons, the engine status, and the write-only Brave
 * Search API key (kept in the system keychain; it can be replaced or removed, never read back).
 */
@Component({
  selector: 'imt-ai-settings-extras',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (status.available()) {
      <div class="actions">
        <button type="button" class="btn" data-testid="ai-settings-setup" (click)="launcher.openSetup()" i18n="@@ai.settings.setup">Set up AI assistant…</button>
        <button type="button" class="btn" data-testid="ai-settings-models" (click)="launcher.openModels()" i18n="@@ai.settings.models">Manage models…</button>
      </div>
      <p class="muted" data-testid="ai-settings-status">{{ statusLine() }}</p>
      <div class="key">
        <label for="ai-brave-key" i18n="@@ai.settings.braveKey">Brave Search API key</label>
        <div class="row">
          <input
            id="ai-brave-key"
            class="input mono"
            type="password"
            autocomplete="off"
            spellcheck="false"
            [placeholder]="keyPlaceholder()"
            [value]="draft()"
            (input)="draft.set($any($event.target).value)"
          />
          <button type="button" class="btn" [disabled]="!draft().trim() || busy()" (click)="saveKey()" i18n="@@ai.settings.saveKey">Save key</button>
          @if (keySet()) {
            <button type="button" class="btn ghost" [disabled]="busy()" (click)="clearKey()" i18n="@@ai.settings.removeKey">Remove key</button>
          }
        </div>
        <p class="muted small" data-testid="ai-key-state">
          {{ keyStateText() }}
        </p>
        @if (keyError()) {
          <p class="err" role="alert">{{ keyError() }}</p>
        }
      </div>
    } @else {
      <p class="note" role="note" data-testid="ai-settings-unavailable">
        {{ unavailableText() }}
      </p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
    p { margin: 0; }
    .actions, .row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .key { display: flex; flex-direction: column; gap: 4px; }
    .key input { flex: 1; min-width: 200px; }
    .small { font-size: 12px; }
    .note { color: var(--muted); }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class AiSettingsExtras implements OnInit {
  private readonly backend = inject(AiBackend);
  protected readonly launcher = inject(AiLauncher);
  protected readonly status = inject(AiStatusStore);

  readonly keySet = signal(false);
  readonly draft = signal('');
  readonly busy = signal(false);
  readonly keyError = signal<string | null>(null);

  protected readonly reason = computed(() => this.status.status()?.reason ?? this.status.error() ?? '');
  protected readonly keyPlaceholder = computed(() =>
    this.keySet()
      ? $localize`:@@ai.settings.key.placeholderSet:A key is stored — paste a new one to replace it`
      : $localize`:@@ai.settings.key.placeholder:Paste your key`,
  );
  protected readonly keyStateText = computed(() =>
    this.keySet()
      ? $localize`:@@ai.settings.key.stateSet:A key is stored in the system keychain. It is used only for searches you approve and cannot be shown again.`
      : $localize`:@@ai.settings.key.stateNone:No key is stored. It is used only for searches you approve and cannot be shown again.`,
  );
  protected readonly unavailableText = computed(() => {
    const reason = this.reason();
    return reason
      ? $localize`:@@ai.settings.unavailableReason:The AI assistant is not available in this run: ${reason}:reason:`
      : $localize`:@@ai.settings.unavailable:The AI assistant is not available in this run.`;
  });
  protected readonly statusLine = computed(() => {
    const s = this.status.status();
    if (!s) return '';
    if (s.state === 'noModel') return $localize`:@@ai.settings.status.noModel:No model is set up yet.`;
    const model = s.model ?? $localize`:@@ai.settings.status.modelNone:none`;
    const parts = [$localize`:@@ai.settings.status.model:Model: ${model}:model:`];
    if (s.backend) parts.push(s.backend);
    if (s.rssBytes) parts.push($localize`:@@ai.settings.status.memory:${formatBytes(s.rssBytes)}:size: in memory`);

    if (s.reason) parts.push(s.reason);
    return parts.join(' · ');
  });

  ngOnInit(): void {
    void this.refreshKey();
  }

  private async refreshKey(): Promise<void> {
    try {
      this.keySet.set(await this.backend.braveKeySet());
    } catch {
      this.keySet.set(false);
    }
  }

  async saveKey(): Promise<void> {
    this.busy.set(true);
    this.keyError.set(null);
    try {
      await this.backend.setBraveKey(this.draft().trim());
      this.draft.set('');
      await this.refreshKey();
    } catch (e) {
      this.keyError.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  async clearKey(): Promise<void> {
    this.busy.set(true);
    this.keyError.set(null);
    try {
      await this.backend.clearBraveKey();
      await this.refreshKey();
    } catch (e) {
      this.keyError.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }
}
