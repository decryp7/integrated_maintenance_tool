import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatBytes } from '../ai-format';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiUpdatesPanel } from '../updates/ai-updates-panel';

/** Settings → AI, above the fields: set-up and model buttons, the engine status and the update checks. */
@Component({
  selector: 'debrief-ai-settings-extras',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiUpdatesPanel],
  template: `
    @if (status.available()) {
      <div class="actions">
        <button type="button" class="btn" data-testid="ai-settings-setup" (click)="launcher.openSetup()" i18n="@@ai.settings.setup">Set up AI assistant…</button>
        <button type="button" class="btn" data-testid="ai-settings-models" (click)="launcher.openModels()" i18n="@@ai.settings.models">Manage models…</button>
      </div>
      <p class="muted" data-testid="ai-settings-status">{{ statusLine() }}</p>
      @if (serverLine(); as line) {
        <p class="muted" data-testid="ai-settings-server">{{ line }}</p>
      }
      <debrief-ai-updates-panel />
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
  /** What the running llama-server reported about itself (diagnostics). */
  protected readonly serverLine = computed(() => {
    const v = this.status.status()?.server;
    if (!v) return '';
    const build = v.build ?? $localize`:@@ai.settings.server.unknownBuild:unknown build`;
    const parts = [$localize`:@@ai.settings.server.build:llama-server ${build}:build:`];
    if (v.context !== null) parts.push($localize`:@@ai.settings.server.context:${v.context}:tokens: tokens of context`);
    if (v.vision === true) parts.push($localize`:@@ai.settings.server.vision:reads images`);
    if (v.vision === false) parts.push($localize`:@@ai.settings.server.noVision:text only`);
    parts.push(
      v.mediaMarker
        ? $localize`:@@ai.settings.server.marker:image marker ${v.mediaMarker}:marker:`
        : $localize`:@@ai.settings.server.defaultMarker:default image marker`,
    );
    return parts.join(' · ');
  });
}
