import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject } from '@angular/core';
import { formatBytes, formatEta, formatRate } from '../shared/format/units';
import { transferSpeed, TransferSpeed } from '../shared/format/transfer-speed';
import { DIALOG_REF } from '../ui/dialog.service';
import { UpdatesStore } from './updates.store';
import { UpdateError, UpdateInfo } from './updates.types';
import { activeLocale } from '../i18n/locale';

/** Errors after which downloading again makes sense (the engine refuses the others too). */
const RETRY_DOWNLOAD = new Set(['offline', 'unreachable', 'diskFull', 'installFailed']);

/** "1.2 MB of 48 MB". */
export function progressText(downloaded: number, total: number | null, speed?: TransferSpeed): string {
  const base = total ? $localize`:@@updates.progress:${formatBytes(downloaded)}:done: of ${formatBytes(total)}:total:` : formatBytes(downloaded);
  return [base, formatRate(speed?.rate), formatEta(speed?.etaSecs)].filter(Boolean).join(' · ');
}

/** A release date for people ("17 Oct 2026"), or '' when absent or invalid. */
export function releaseDate(iso: string | null): string {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(activeLocale(), { day: 'numeric', month: 'short', year: 'numeric' });
}

/**
 * Help ▸ Check for updates…: the whole flow in one dialog. Release notes are plain text interpolation
 * (never `innerHTML`); the engine bounds and cleans them.
 */
@Component({
  selector: 'imt-updates-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="current faint" i18n="@@updates.current">You have IMT {{ store.status()?.currentVersion ?? '…' }}.</p>
    @switch (phase()?.kind) {
      @case ('unavailable') {
        <p class="msg" data-testid="updates-unavailable" i18n="@@updates.unavailable.title">Updates are not available in this build.</p>
        <p class="next" i18n="@@updates.unavailable.next">
          This build was made without an update signing key, so it cannot verify updates. Download new versions from the releases page.
        </p>
        <div class="actions"><button class="btn" type="button" (click)="store.openReleases()" i18n="@@updates.openReleases">Open releases page</button></div>
      }
      @case ('checking') {
        <p class="msg" role="status" i18n="@@updates.checking">Checking for updates…</p>
      }
      @case ('upToDate') {
        <p class="msg" role="status" data-testid="updates-up-to-date" i18n="@@updates.upToDate">IMT is up to date.</p>
        <div class="actions"><button class="btn" type="button" (click)="store.check()" i18n="@@updates.checkAgain">Check again</button></div>
      }
      @case ('available') {
        <ng-container [ngTemplateOutlet]="release" [ngTemplateOutletContext]="{ $implicit: info() }" />
        <div class="actions">
          <button class="btn primary" type="button" (click)="store.download()" i18n="@@updates.download">Download</button>
          <button class="btn ghost" type="button" (click)="close()" i18n="@@updates.later">Later</button>
        </div>
      }
      @case ('downloading') {
        <p class="msg" i18n="@@updates.downloading">Downloading IMT {{ info()?.version }}…</p>
        <progress max="1" [value]="store.progress() ?? 0" aria-label="Download progress" i18n-aria-label="@@updates.downloading.label" data-testid="updates-progress"></progress>
        <p class="faint" role="status">{{ progress() }}</p>
        <div class="actions"><button class="btn" type="button" (click)="store.cancel()" i18n="@@updates.cancel">Cancel</button></div>
      }
      @case ('ready') {
        <p class="msg" role="status" data-testid="updates-ready" i18n="@@updates.ready">IMT {{ info()?.version }} is downloaded and its signature verified.</p>
        <label class="quit" i18n="@@updates.installOnQuit">
          <input type="checkbox" [checked]="installOnQuit()" (change)="store.setInstallOnQuit($any($event.target).checked)" />
          Install when IMT quits instead
        </label>
        <div class="actions">
          <button class="btn primary" type="button" (click)="store.restart()" i18n="@@updates.restart">Restart to update</button>
          <button class="btn ghost" type="button" (click)="store.cancel()" i18n="@@updates.discard">Discard</button>
          <button class="btn ghost" type="button" (click)="close()" i18n="@@updates.later">Later</button>
        </div>
      }
      @case ('installing') {
        <p class="msg" role="status" i18n="@@updates.installing">Restarting to install IMT {{ info()?.version }}…</p>
      }
      @case ('failed') {
        <ng-container [ngTemplateOutlet]="failure" [ngTemplateOutletContext]="{ $implicit: failed() }" />
        <div class="actions">
          @if (canRetryDownload()) {
            <button class="btn primary" type="button" (click)="store.download()" i18n="@@updates.retryDownload">Try the download again</button>
          } @else {
            <button class="btn primary" type="button" (click)="store.check()" i18n="@@updates.checkAgain">Check again</button>
          }
          <button class="btn ghost" type="button" (click)="store.openReleases()" i18n="@@updates.openReleases">Open releases page</button>
        </div>
      }
      @default {
        <p class="msg" role="status" i18n="@@updates.checking">Checking for updates…</p>
      }
    }
    @if (store.error(); as e) {
      <ng-container [ngTemplateOutlet]="failure" [ngTemplateOutletContext]="{ $implicit: e }" />
    }
    @if (store.available()) {
      <p class="auto faint" data-testid="updates-auto">
        {{ autoCheckText() }}
      </p>
    }

    <ng-template #release let-info>
      <p class="msg" data-testid="updates-available" i18n="@@updates.available">IMT {{ info.version }} is available.</p>
      <p class="faint meta">
        @if (date(info)) {
          <ng-container i18n="@@updates.released">Released {{ date(info) }}</ng-container>
        }
        @if (info.size) {
          · {{ size(info) }}
        }
      </p>
      @if (info.notes) {
        <div class="notes" tabindex="0" role="region" aria-label="Release notes" i18n-aria-label="@@updates.notes.label" data-testid="updates-notes">{{ info.notes }}</div>
      }
    </ng-template>

    <ng-template #failure let-e>
      <div class="err" role="alert" data-testid="updates-error" [attr.data-kind]="e.kind">
        <p class="msg">{{ e.message }}</p>
        @if (e.nextStep) {
          <p class="next">{{ e.nextStep }}</p>
        }
        @if (e.detail) {
          <p class="detail faint">{{ e.detail }}</p>
        }
      </div>
    </ng-template>
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
    p { margin: 0; }
    .current { font-size: var(--fs-sm); }
    .msg { font-weight: 600; }
    .next { color: var(--text-2); line-height: 1.45; }
    .meta { font-size: var(--fs-sm); }
    .notes { max-height: 240px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; padding: 8px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-2); font-size: var(--fs-md); line-height: 1.45; }
    progress { width: 100%; }
    .quit { display: flex; gap: 6px; align-items: center; font-size: var(--fs-md); }
    .actions { display: flex; gap: 8px; flex-wrap: wrap; }
    .err { display: flex; flex-direction: column; gap: 4px; padding: 8px 10px; border: 1px solid var(--border-2); border-inline-start: 3px solid var(--lvl-error); border-radius: 6px; background: var(--bg-3); }
    .err .msg { color: var(--danger-text); }
    .detail { font-family: var(--font-mono); font-size: var(--fs-xs); overflow-wrap: anywhere; }
    .auto { font-size: var(--fs-xs); line-height: 1.4; border-top: 1px solid var(--border); padding-top: 8px; }
  `,
})
export class UpdatesDialog {
  protected readonly store = inject(UpdatesStore);
  private readonly ref = inject(DIALOG_REF, { optional: true });

  protected readonly phase = this.store.phase;
  protected readonly info = computed<UpdateInfo | null>(() => {
    const p = this.phase();
    return p && 'info' in p ? p.info : null;
  });
  protected readonly failed = computed<UpdateError | null>(() => {
    const p = this.phase();
    return p?.kind === 'failed' ? p.error : null;
  });
  protected readonly installOnQuit = computed(() => {
    const p = this.phase();
    return p?.kind === 'ready' && p.installOnQuit;
  });
  protected readonly autoCheckText = computed(() => {
    const s = this.store.status();
    const host = s?.endpointHost;
    return s?.autoCheck
      ? $localize`:@@updates.auto.on:Automatic checks are on (Settings ▸ Updates). A check contacts only ${host}:host: and sends no identifier or usage data.`
      : $localize`:@@updates.auto.off:Automatic checks are off (Settings ▸ Updates). A check contacts only ${host}:host: and sends no identifier or usage data.`;
  });
  protected readonly canRetryDownload = computed(() => {
    const p = this.phase();
    return p?.kind === 'failed' && !!p.info && RETRY_DOWNLOAD.has(p.error.kind);
  });
  private readonly speed = transferSpeed(() => {
    const p = this.phase();
    return p?.kind === 'downloading' && p.total ? { bytes: p.downloaded, total: p.total } : null;
  });
  protected readonly progress = computed(() => {
    const p = this.phase();
    return p?.kind === 'downloading' ? progressText(p.downloaded, p.total, this.speed()) : '';
  });

  constructor() {
    this.store.setDialogOpen(true);
    inject(DestroyRef).onDestroy(() => this.store.setDialogOpen(false));
  }

  protected date(info: UpdateInfo): string {
    return releaseDate(info.pubDate);
  }

  protected size(info: UpdateInfo): string {
    return info.size ? formatBytes(info.size) : '';
  }

  protected close(): void {
    this.ref?.close();
  }
}
