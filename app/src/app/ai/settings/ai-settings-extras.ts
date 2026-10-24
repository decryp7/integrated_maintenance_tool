import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { formatBytes } from '../ai-format';
import { AiBackend, toAiError } from '../ai-backend';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { engineLines, groupCode } from '../pitwall-format';
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
      @if (engine(); as e) {
        <section class="engine" aria-label="AI engine" i18n-aria-label="@@ai.settings.engine">
          @for (line of lines(); track $index) {
            <p [attr.data-testid]="$index === 0 ? 'ai-engine-status' : null" [class.muted]="$index > 0">{{ line }}</p>
          }
          @if (e.pitwall.pairing; as pairing) {
            <p class="code" data-testid="ai-pitwall-code" aria-label="Pairing code" i18n-aria-label="@@ai.settings.pitwall.codeLabel">{{ group(pairing.code) }}</p>
            <p class="muted" i18n="@@ai.settings.pitwall.approve">Approve Debrief in Pitwall ▸ Apps & keys and check it shows this code.</p>
          }
          <div class="actions">
            @if (e.pitwall.found === 'running' && !e.pitwall.paired && !e.pitwall.pairing) {
              <button type="button" class="btn" data-testid="ai-pitwall-connect" [disabled]="busy()" (click)="connect()" i18n="@@ai.settings.pitwall.connect">Connect to Pitwall…</button>
            }
            @if (e.pitwall.pairing) {
              <button type="button" class="btn" data-testid="ai-pitwall-cancel" [disabled]="busy()" (click)="cancel()" i18n="@@ai.settings.pitwall.cancel">Cancel</button>
            }
            @if (e.pitwall.paired) {
              <button type="button" class="btn" data-testid="ai-pitwall-forget" [disabled]="busy()" (click)="forget()" i18n="@@ai.settings.pitwall.forget">Forget Pitwall</button>
            }
            @if (e.active === null) {
              <button type="button" class="btn" data-testid="ai-use-built-in" [disabled]="busy()" (click)="useBuiltIn()" i18n="@@ai.settings.pitwall.useBuiltIn">Use the built-in engine</button>
            }
          </div>
          @if (engineError(); as err) {
            <p class="err" role="alert" data-testid="ai-engine-error">{{ err }}</p>
          }
        </section>
      }
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
    .engine { display: flex; flex-direction: column; gap: 6px; }
    .code { font-size: var(--fs-lg, 1.5em); font-weight: 600; letter-spacing: 0.15em; font-variant-numeric: tabular-nums; }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class AiSettingsExtras {
  protected readonly launcher = inject(AiLauncher);
  protected readonly status = inject(AiStatusStore);
  private readonly backend = inject(AiBackend);

  protected readonly engine = computed(() => this.status.status()?.engine ?? null);
  protected readonly lines = computed(() => {
    const e = this.engine();
    return e ? engineLines(e) : [];
  });
  protected readonly group = groupCode;
  protected readonly busy = signal(false);
  protected readonly engineError = signal<string | null>(null);

  connect(): Promise<void> {
    return this.act(async () => void (await this.backend.pitwallConnect(false)));
  }
  cancel(): Promise<void> {
    return this.act(async () => void (await this.backend.pitwallCancel()));
  }
  forget(): Promise<void> {
    return this.act(async () => void (await this.backend.pitwallForget()));
  }
  useBuiltIn(): Promise<void> {
    return this.act(async () => void (await this.backend.useBuiltInEngine()));
  }

  /** Runs one Pitwall command; the new status arrives on the status event, a rejection is shown inline. */
  private async act(run: () => Promise<void>): Promise<void> {
    this.busy.set(true);
    this.engineError.set(null);
    try {
      await run();
    } catch (e) {
      this.engineError.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

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
