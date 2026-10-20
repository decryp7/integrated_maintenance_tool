import { ChangeDetectionStrategy, Component, inject, OnInit, signal } from '@angular/core';
import { formatBytes } from '../ai-format';
import { AiUpdate, AiUpdateKind, AiUpdateKindStatus, AiUpdatePolicy } from '../ai.types';
import { AiManualDownload } from '../manual/manual-download';
import { AiUpdatesStore } from './ai-updates.store';

/** The title of a kind's group. */
export function kindTitle(kind: AiUpdateKind): string {
  return kind === 'runtime'
    ? $localize`:@@ai.updates.kind.runtime:Inference runtime`
    : $localize`:@@ai.updates.kind.model:Models`;
}

export function policyLabel(p: AiUpdatePolicy): string {
  switch (p) {
    case 'automatic':
      return $localize`:@@ai.updates.policy.automatic:Automatic`;
    case 'ask':
      return $localize`:@@ai.updates.policy.ask:Ask`;
    case 'manual':
      return $localize`:@@ai.updates.policy.manual:Manual`;
  }
}

/** "Checked 20 Oct, 14:05" or "Never checked". */
export function lastCheckText(k: AiUpdateKindStatus, locale?: string): string {
  if (k.lastCheck === null) return $localize`:@@ai.updates.never:Never checked`;
  const when = new Date(k.lastCheck).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' });
  return $localize`:@@ai.updates.checked:Checked ${when}:when:`;
}

/** "Next check 21 Oct, 14:05", "Checks only when you choose Check now" (Manual). */
export function nextCheckText(k: AiUpdateKindStatus, locale?: string): string {
  if (k.nextCheck === null) return $localize`:@@ai.updates.manualOnly:Checks only when you choose Check now`;
  const when = new Date(k.nextCheck).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' });
  return $localize`:@@ai.updates.next:Next check ${when}:when:`;
}

/** "b11669 → b11700 · 32 MB". */
export function updateLine(u: AiUpdate): string {
  return `${u.from} → ${u.to} · ${formatBytes(u.downloadBytes)}`;
}

/**
 * Settings → AI → Updates: per kind the policy (set with the fields below), the last check and its quiet
 * failure (with "Download manually" when the network blocks downloads), the updates on offer with Update and
 * Skip this version, and "Check now".
 */
@Component({
  selector: 'imt-ai-updates-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiManualDownload],
  template: `
    <section class="updates" aria-labelledby="ai-updates-title" data-testid="ai-updates">
      <div class="head">
        <h4 id="ai-updates-title" i18n="@@ai.updates.title">Updates</h4>
        <span class="spacer"></span>
        <button
          type="button"
          class="btn"
          data-testid="ai-updates-check"
          [disabled]="store.busy() || store.checking()"
          (click)="store.check()"
          i18n="@@ai.updates.checkNow"
        >
          Check now
        </button>
      </div>
      @if (store.checking()) {
        <p class="muted" role="status" data-testid="ai-updates-checking" i18n="@@ai.updates.checking">Checking for updates…</p>
      }
      @if (store.error(); as e) {
        <p class="err" role="alert" data-testid="ai-updates-error">
          {{ e }}
          <button type="button" class="btn ghost" (click)="store.load()" i18n="@@ai.updates.retry">Retry</button>
        </p>
      }
      @for (k of store.kinds(); track k.kind) {
        <div class="kind" [attr.data-testid]="'ai-updates-' + k.kind">
          <div class="line">
            <strong>{{ kindTitle(k.kind) }}</strong>
            <span class="badge" [attr.data-testid]="'ai-updates-policy-' + k.kind">{{ policyLabel(k.policy) }}</span>
            <span class="muted">{{ lastCheckText(k) }} · {{ nextCheckText(k) }}</span>
          </div>
          @if (k.lastError; as err) {
            <p class="warn" role="note" [attr.data-testid]="'ai-updates-failed-' + k.kind">{{ err }}</p>
            @if (k.manualSuggested && firstOf(k); as u) {
              @if (manualFor() === u.key) {
                <imt-ai-manual-download [kind]="u.kind" [id]="u.id" />
              } @else {
                <button type="button" class="btn ghost" (click)="manualFor.set(u.key)" data-testid="ai-updates-manual" i18n="@@ai.updates.manual">Download manually…</button>
              }
            }
          }
          @if (k.updates.length) {
            <ul class="list" [attr.aria-label]="kindTitle(k.kind)">
              @for (u of k.updates; track u.key) {
                <li class="update" [attr.data-testid]="'ai-update-' + u.key">
                  <span class="name">{{ u.name }}</span>
                  <span class="mono muted">{{ updateLine(u) }}</span>
                  @if (u.installing) {
                    <span class="badge" i18n="@@ai.updates.installing">Installing…</span>
                  } @else if (u.waiting) {
                    <span class="badge" i18n="@@ai.updates.waiting">Waits for the chat to finish</span>
                  } @else if (u.skipped) {
                    <span class="badge" i18n="@@ai.updates.skipped">Skipped</span>
                  }
                  <span class="spacer"></span>
                  @if (!u.installing) {
                    <button type="button" class="btn primary" [disabled]="store.busy()" (click)="store.apply(u.key)" [attr.aria-label]="updateLabel(u)" data-testid="ai-update-apply" i18n="@@ai.updates.update">Update</button>
                    @if (!u.skipped) {
                      <button type="button" class="btn ghost" [disabled]="store.busy()" (click)="store.skip(u.key)" [attr.aria-label]="skipLabel(u)" data-testid="ai-update-skip" i18n="@@ai.updates.skip">Skip this version</button>
                    }
                  }
                </li>
              }
            </ul>
          } @else if (k.lastCheck !== null && !k.lastError) {
            <p class="muted" [attr.data-testid]="'ai-updates-current-' + k.kind" i18n="@@ai.updates.upToDate">Up to date.</p>
          }
        </div>
      }
    </section>
  `,
  styles: `
    :host { display: block; }
    .updates { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 6px; }
    h4 { margin: 0; font-size: var(--fs-md); }
    p { margin: 0; }
    .head, .line, .update { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .spacer { flex: 1; }
    .kind { display: flex; flex-direction: column; gap: 6px; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .muted { color: var(--text-2); }
    .mono { font-family: var(--font-mono, monospace); font-size: var(--fs-sm); }
    .badge { font-size: var(--fs-xs); padding: 1px 6px; border-radius: 8px; background: var(--bg-3); color: var(--text-2); }
    .warn { color: var(--lvl-warn); overflow-wrap: anywhere; }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class AiUpdatesPanel implements OnInit {
  protected readonly store = inject(AiUpdatesStore);
  protected readonly kindTitle = kindTitle;
  protected readonly policyLabel = policyLabel;
  protected readonly lastCheckText = lastCheckText;
  protected readonly nextCheckText = nextCheckText;
  protected readonly updateLine = updateLine;
  /** The update whose files are shown for downloading by hand. */
  protected readonly manualFor = signal<string | null>(null);

  ngOnInit(): void {
    void this.store.load();
  }

  protected firstOf(k: AiUpdateKindStatus): AiUpdate | null {
    return k.updates[0] ?? null;
  }

  protected updateLabel(u: AiUpdate): string {
    return $localize`:@@ai.updates.updateLabel:Update ${u.name}:name: to ${u.to}:version:`;
  }

  protected skipLabel(u: AiUpdate): string {
    return $localize`:@@ai.updates.skipLabel:Skip this version of ${u.name}:name: (${u.to}:version:)`;
  }
}
