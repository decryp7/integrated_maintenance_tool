import { ChangeDetectionStrategy, Component, inject, Injectable, Injector, input, output } from '@angular/core';
import { DumpFinding } from '../core/api.types';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { AnalysisState } from '../analysis/analysis-state';
import { ANALYSIS_STYLES } from '../analysis/analysis-styles';
import { AsyncResource } from '../analysis/async-resource';
import { SeverityBadge } from '../live/severity-badge';
import { DumpsBackend } from './dumps-backend';

/** The AI chat view (provide-ai.ts `AI_CHAT_VIEW`). */
const AI_CHAT_VIEW = 'ai-chat';

/** `12.3 MB`, `-2.0 KB`, `512 bytes` (display only; the engine computes the values). */
export function formatBytes(n: number): string {
  const sign = n < 0 ? '-' : '';
  const v = Math.abs(n);
  if (v >= 1 << 30) return `${sign}${(v / (1 << 30)).toFixed(1)} GB`;
  if (v >= 1 << 20) return `${sign}${(v / (1 << 20)).toFixed(1)} MB`;
  if (v >= 1 << 10) return `${sign}${(v / (1 << 10)).toFixed(1)} KB`;
  return $localize`:@@dumps.bytes:${sign + v}:n: bytes`;
}

/** A signed change: `+1.2 MB`. */
export function formatDelta(n: number): string {
  return n > 0 ? `+${formatBytes(n)}` : formatBytes(n);
}

/** Layout shared by the dump views (on top of the analysis views'); colours are app tokens. */
export const DUMPS_STYLES =
  ANALYSIS_STYLES +
  `
  select, input[type='search'], input[type='text'] { font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--bg-3); border: 1px solid var(--border-2); border-radius: 4px; padding: 2px 6px; height: 24px; }
  select:focus-visible, input:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -1px; }
  label.inline { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
  .mono { font-family: var(--font-mono); }
  .tag { font-family: var(--font-mono); font-size: var(--fs-2xs); padding: 0 4px; border-radius: 3px; border: 1px solid var(--border-3); color: var(--text-2); white-space: nowrap; }
  .tag.culprit { color: var(--danger-text); border-color: var(--danger-text); }
  .faint { color: var(--muted); }
  .tabs { display: flex; gap: 2px; padding: 0 12px; border-bottom: 1px solid var(--border); }
  .tabs [role='tab'] { all: unset; cursor: pointer; padding: 6px 10px; color: var(--muted); border-bottom: 2px solid transparent; }
  .tabs [role='tab'][aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
  .tabs [role='tab']:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  .finding { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 8px 10px; margin-bottom: 8px; }
  .finding .head { display: flex; align-items: center; gap: 8px; }
  .finding .title { color: var(--text); }
  .finding .detail { margin: 4px 0 6px; color: var(--text-2); }
  .finding .foot { display: flex; align-items: center; gap: 8px; }
  .notice { padding: 4px 12px; font-size: var(--fs-sm); color: var(--muted); border-bottom: 1px solid var(--border); }
  .err { color: var(--danger-text); }
  tr.sel td { background: var(--bg-sel); }
  tr.clickable { cursor: pointer; }
  tr.clickable:hover td { background: var(--bg-hover); }
  tr.framework td { color: var(--muted); }
  tr.culprit td { background: var(--bg-3); }
`;

/** "Explain with AI": asks the engine for its compact, redacted summary of the documents and sends it to the chat. */
@Injectable({ providedIn: 'root' })
export class DumpsExplainer {
  private readonly injector = inject(Injector);
  private readonly backend = inject(DumpsBackend);

  async explain(ids: number[]): Promise<boolean> {
    const text = await this.backend.explain({ ids });
    this.injector.get(COMMAND_CONTEXT).showView(AI_CHAT_VIEW);
    const { ChatStore } = await import('../ai/chat/chat.store');
    return this.injector.get(ChatStore).send(text);
  }
}

/** The engine's findings, most severe first, each with a link to its evidence and "Explain with AI". */
@Component({
  selector: 'imt-dump-findings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, SeverityBadge],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@dumps.findings.title">Findings</span>
      <span class="spacer"></span>
      <button class="btn" type="button" data-testid="dumps-explain" [disabled]="!res().data()?.length" (click)="explain.emit()" i18n="@@dumps.findings.explain">Explain with AI</button>
    </div>
    <imt-analysis-state [loading]="res().loading()" [error]="res().error()" [retryable]="res().retryable()" [hasData]="!!res().data()" [empty]="res().data()?.length === 0" [emptyText]="emptyText()" (retry)="res().retry()">
      <ul class="list body" aria-label="Findings" i18n-aria-label="@@dumps.findings.listLabel">
        @for (f of res().data() ?? []; track $index) {
          <li class="finding" [attr.data-kind]="f.kind">
            <div class="head">
              <imt-severity-badge [severity]="f.severity" />
              <strong class="title">{{ f.title }}</strong>
            </div>
            <p class="detail">{{ f.detail }}</p>
            @if (hasEvidence(f)) {
              <div class="foot">
                <button class="btn" type="button" (click)="evidence.emit(f)" [attr.aria-label]="evidenceLabel(f)" i18n="@@dumps.findings.show">Show</button>
              </div>
            }
          </li>
        }
      </ul>
    </imt-analysis-state>
  `,
  styles: [DUMPS_STYLES, `:host { flex: 1; }`],
})
export class DumpFindings {
  readonly res = input.required<AsyncResource<DumpFinding[]>>();
  readonly emptyText = input($localize`:@@dumps.findings.empty:No findings.`);
  readonly evidence = output<DumpFinding>();
  readonly explain = output<void>();

  protected hasEvidence(f: DumpFinding): boolean {
    return f.evidence.threadId !== null || f.evidence.stack !== null || f.evidence.threads.length > 0;
  }

  protected evidenceLabel(f: DumpFinding): string {
    return $localize`:@@dumps.findings.showLabel:Show the evidence of ${f.title}:title:`;
  }
}

/** What to show when no document of the view's kind is open. */
@Component({
  selector: 'imt-no-dump',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="empty" role="status">
      <p>{{ text() }}</p>
      <button class="btn" type="button" (click)="open.emit()">{{ action() }}</button>
    </div>
  `,
  styles: `
    :host { flex: 1; display: flex; }
    .empty { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 8px; color: var(--muted); font-size: var(--fs-sm); }
  `,
})
export class NoDump {
  readonly text = input.required<string>();
  readonly action = input.required<string>();
  readonly open = output<void>();
}
