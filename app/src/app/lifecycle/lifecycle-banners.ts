import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Icon } from '../shared/icon';
import { ProjectService } from '../projects/project.service';
import { baseName } from '../tools/tool-errors';
import { DiagnosticsExport } from '../diagnostics/diagnostics-export.service';
import { LifecycleService } from './lifecycle.service';
import { BundleBanner } from '../share/bundle/bundle-banner';
import { RedactionBanner } from '../share/redaction/redaction-banner';

/** Banners under the title bar: safe mode, crash recovery and project paths that were not found. */
@Component({
  selector: 'debrief-lifecycle-banners',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, BundleBanner, RedactionBanner],
  host: { style: 'display: contents' },
  template: `
    @if (life.safeMode()) {
      <div class="banner warn" role="status" data-testid="safe-mode-banner">
        <debrief-icon name="alert" [size]="14" />
        <span class="msg" i18n="@@lifecycle.banners.safeMode">Running in safe mode: plugins and saved profile are off.</span>
        <button type="button" class="btn" (click)="life.restartNormally()" i18n="@@lifecycle.banners.restartNormally">Restart normally</button>
        <button type="button" class="btn" [disabled]="diagnostics.busy()" (click)="diagnostics.save()" data-testid="save-diagnostics" i18n="@@lifecycle.banners.saveDiagnostics">Save diagnostics…</button>
      </div>
    }
    @if (life.showCrashBanner()) {
      <div class="banner warn" role="alert" data-testid="crash-banner">
        <debrief-icon name="alert" [size]="14" />
        <span class="msg" i18n="@@lifecycle.banners.crashed">Debrief didn't close properly last time.</span>
        <button type="button" class="btn primary" [disabled]="!life.sessionAvailable()" (click)="life.restoreLastSession()" i18n="@@lifecycle.banners.restore">Restore last session</button>
        <button type="button" class="btn" (click)="life.startInSafeMode()" i18n="@@lifecycle.banners.startSafeMode">Start in safe mode</button>
        <button type="button" class="btn" [disabled]="diagnostics.busy()" (click)="diagnostics.save()" data-testid="save-diagnostics" i18n="@@lifecycle.banners.saveDiagnostics">Save diagnostics…</button>
        <button type="button" class="btn ghost" (click)="life.dismissCrash()" i18n="@@lifecycle.banners.dismissCrash">Dismiss</button>
      </div>
    }
    @if (diagnostics.status(); as s) {
      <div class="banner" [class.warn]="s.kind === 'failed'" [class.info]="s.kind === 'saved'" [attr.role]="s.kind === 'failed' ? 'alert' : 'status'" data-testid="diagnostics-export-status">
        <debrief-icon [name]="s.kind === 'failed' ? 'alert' : 'check'" [size]="14" />
        <span class="msg" [title]="s.text">{{ s.text }}</span>
        <button type="button" class="btn ghost" (click)="diagnostics.dismiss()" i18n="@@lifecycle.banners.dismissDiagnostics">Dismiss</button>
      </div>
    }
    @if (projects.missing().length; as n) {
      <div class="banner info" role="status" data-testid="missing-banner">
        <debrief-icon name="alert" [size]="14" />
        <span class="msg" i18n="@@lifecycle.banners.missing">{n, plural, =1 {{{ n }} path} other {{{ n }} paths}} of this project could not be found: <span class="mono" [title]="projects.missing().join('\\n')">{{ names() }}</span></span>
        <button type="button" class="btn ghost" (click)="projects.dismissMissing()" i18n="@@lifecycle.banners.dismissMissing">Dismiss</button>
      </div>
    }
    <debrief-bundle-banner />
    <debrief-redaction-banner />
  `,
  styles: `
    .banner { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; font-size: var(--fs-sm); border-bottom: 1px solid var(--border); background: var(--bg-3); color: var(--text); }
    .banner.warn { border-inline-start: 3px solid var(--lvl-warn); }
    .banner.warn debrief-icon { color: var(--warn-text); }
    .banner.info { border-inline-start: 3px solid var(--accent); }
    .msg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  `,
})
export class LifecycleBanners {
  protected readonly life = inject(LifecycleService);
  protected readonly projects = inject(ProjectService);
  protected readonly diagnostics = inject(DiagnosticsExport);

  protected names(): string {
    const names = this.projects.missing().map(baseName);
    return names.length > 3 ? $localize`:@@lifecycle.banners.andMore:${names.slice(0, 3).join(', ')}:names: and ${names.length - 3}:count: more` : names.join(', ');
  }
}
