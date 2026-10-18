import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatBytes } from '../ai-format';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';

/** Settings → AI, above the fields: set-up and model buttons and the engine status. */
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
    } @else {
      <p class="note" role="note" data-testid="ai-settings-unavailable">
        {{ unavailableText() }}
      </p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
    p { margin: 0; }
    .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .note { color: var(--muted); }
  `,
})
export class AiSettingsExtras {
  protected readonly launcher = inject(AiLauncher);
  protected readonly status = inject(AiStatusStore);

  protected readonly reason = computed(() => this.status.status()?.reason ?? this.status.error() ?? '');
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
}
