import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { ProfileVersionInfo } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { formatFull } from '../../core/format';
import { describeError } from '../../core/logger';
import { UxBackend } from '../../core/ux-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { SettingsStore } from '../../settings/settings.store';
import { ProfileUndo } from '../../undo/profile-undo';
import { ProfileDraft } from '../profile-draft';
import { lineDiff, stampToMs } from './line-diff';

/**
 * The profile's saved versions (the shell keeps the last 20): pick one to see how it differs from the
 * current profile, and restore it through the engine's validated save (undoable).
 */
@Component({
  selector: 'debrief-profile-history',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (error(); as e) {
      <div class="state err" role="alert"><ng-container i18n="@@profile.history.loadFailed">Could not load the profile history: {{ e }}</ng-container>&ngsp;<button type="button" class="btn" (click)="load()" i18n="@@profile.history.retry">Retry</button></div>
    } @else if (!versions()) {
      <div class="state muted" role="status" i18n="@@profile.history.loading">Loading history…</div>
    } @else if (!versions()!.length) {
      <div class="state empty">
        <p i18n="@@profile.history.none">No earlier versions yet: every save keeps the previous one, up to 20.</p>
        <p class="muted" i18n="@@profile.history.noneHint">Edit the profile in the Form or TOML view and press Save &amp; apply; the version it replaces appears here.</p>
      </div>
    } @else {
      <div class="cols">
        <ul class="list" aria-label="Saved versions" i18n-aria-label="@@profile.history.versions">
          @for (v of versions(); track v.file) {
            <li>
              <button type="button" class="ver" [class.on]="v.file === selected()" [attr.aria-pressed]="v.file === selected()" (click)="select(v.file)">
                <span class="when">{{ when(v) }}</span>
                <span class="vname">{{ v.name }}</span>
              </button>
            </li>
          }
        </ul>
        <section class="detail" aria-label="Difference from the current profile" i18n-aria-label="@@profile.history.difference">
          @if (!selected()) {
            <p class="muted" i18n="@@profile.history.select">Select a version to compare it with the current profile.</p>
          } @else if (versionError(); as e) {
            <div class="err" role="alert"><ng-container i18n="@@profile.history.versionFailed">Could not read this version: {{ e }}</ng-container>&ngsp;<button type="button" class="btn" (click)="select(selected()!)" i18n="@@profile.history.retry">Retry</button></div>
          } @else if (diff(); as d) {
            <div class="summary" role="status">
              @if (!d.added && !d.removed) {
                <ng-container i18n="@@profile.history.same">This version is the same as the current profile.</ng-container>
              } @else {
                <ng-container i18n="@@profile.history.restoring">Restoring it would remove {d.removed, plural, =1 {{{ d.removed }} line} other {{{ d.removed }} lines}} and add {{ d.added }}.</ng-container>
              }
              @if (d.coarse) { &ngsp;<span class="muted" i18n="@@profile.history.coarse">(too large to align line by line)</span> }
            </div>
            <pre class="diff mono" tabindex="0" aria-label="Line differences: lines starting with minus are removed, with plus are added" i18n-aria-label="@@profile.history.diffLabel">@for (l of d.lines; track $index) {<span [class]="'l ' + l.kind"><span class="sign" aria-hidden="true">{{ sign(l.kind) }}</span><span class="sr">{{ srLabel(l.kind) }}</span>{{ l.text }}
</span>}</pre>
            @if (restoreError(); as e) { <div class="err" role="alert" i18n="@@profile.history.notRestored">Not restored: {{ e }}</div> }
            <div class="foot">
              @if (draft.dirty()) { <span class="muted" i18n="@@profile.history.saveFirst">Save or revert your unsaved edits first.</span> }
              <button type="button" class="btn primary" [disabled]="restoring() || draft.dirty() || (!d.added && !d.removed)" (click)="restore()">@if (restoring()) { <ng-container i18n="@@profile.history.restoringBusy">Restoring…</ng-container> } @else { <ng-container i18n="@@profile.history.restore">Restore this version</ng-container> }</button>
            </div>
          } @else {
            <p class="muted" role="status" i18n="@@profile.history.loadingVersion">Loading version…</p>
          }
        </section>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 12px 14px; gap: 10px; overflow: auto; }
    p { margin: 0 0 6px; }
    .muted { color: var(--muted); font-size: var(--fs-sm); }
    .err { color: var(--danger-text); font-size: var(--fs-sm); }
    .state { padding: 12px 0; }
    .empty p:first-child { color: var(--text); }
    .cols { display: grid; grid-template-columns: 260px 1fr; gap: 14px; min-height: 0; flex: 1; }
    .list { list-style: none; margin: 0; padding: 0; overflow: auto; display: flex; flex-direction: column; gap: 2px; border-right: 1px solid var(--border); padding-right: 8px; }
    .ver { width: 100%; display: flex; flex-direction: column; align-items: flex-start; gap: 1px; padding: 5px 8px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; font: inherit; }
    .ver:hover { background: var(--bg-hover); color: var(--text); }
    .ver.on { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .ver:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .when { font-size: var(--fs-sm); }
    .vname { font-size: var(--fs-xs); color: var(--muted); }
    .detail { display: flex; flex-direction: column; gap: 8px; min-width: 0; min-height: 0; }
    .summary { font-size: var(--fs-sm); color: var(--text-2); }
    .diff { margin: 0; padding: 6px 0; flex: 1; min-height: 120px; max-height: 480px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--code-bg); font-size: var(--fs-xs); line-height: 1.5; }
    .l { display: block; padding: 0 8px; white-space: pre-wrap; word-break: break-word; }
    .l.removed { background: color-mix(in srgb, var(--danger-text) 14%, transparent); }
    .l.added { background: color-mix(in srgb, var(--ok) 16%, transparent); }
    .sign { display: inline-block; width: 14px; color: var(--muted); }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .foot { display: flex; justify-content: flex-end; align-items: center; gap: 10px; }
  `,
})
export class ProfileHistory {
  protected readonly draft = inject(ProfileDraft);
  private readonly ux = inject(UxBackend);
  private readonly backend = inject(DebriefBackend);
  private readonly undo = inject(ProfileUndo);
  private readonly store = inject(WorkspaceStore);
  private readonly settings = inject(SettingsStore);

  protected readonly versions = signal<ProfileVersionInfo[] | null>(null);
  protected readonly error = signal<string | null>(null);
  protected readonly selected = signal<string | null>(null);
  private readonly versionText = signal<string | null>(null);
  protected readonly versionError = signal<string | null>(null);
  protected readonly restoring = signal(false);
  protected readonly restoreError = signal<string | null>(null);
  private seq = 0;

  /** Current profile → selected version: what restoring would change. */
  protected readonly diff = computed(() => {
    const text = this.versionText();
    const current = this.draft.doc()?.toml;
    return text === null || current === undefined ? null : lineDiff(current, text);
  });

  constructor() {
    // Every save adds a version: reload the list when the profile changes.
    effect(() => {
      this.store.profileVersion();
      untracked(() => void this.load());
    });
  }

  async load(): Promise<void> {
    this.error.set(null);
    try {
      const list = await this.ux.profileHistory();
      this.versions.set(list);
      if (this.selected() && !list.some((v) => v.file === this.selected())) {
        this.selected.set(null);
        this.versionText.set(null);
      }
    } catch (e) {
      this.error.set(describeError(e));
    }
  }

  protected async select(file: string): Promise<void> {
    const mine = ++this.seq;
    this.selected.set(file);
    this.versionText.set(null);
    this.versionError.set(null);
    this.restoreError.set(null);
    try {
      const doc = await this.ux.profileVersion(file);
      if (mine === this.seq) this.versionText.set(doc.toml);
    } catch (e) {
      if (mine === this.seq) this.versionError.set(describeError(e));
    }
  }

  protected when(v: ProfileVersionInfo): string {
    const ms = v.savedAtMs ?? stampToMs(v.savedAt);
    return ms === null ? v.savedAt : formatFull(ms, this.store.timezone(), this.settings.dateFormat());
  }

  protected sign(kind: string): string {
    return kind === 'added' ? '+' : kind === 'removed' ? '−' : ' ';
  }

  protected srLabel(kind: string): string {
    return kind === 'added' ? $localize`:@@profile.history.srAdded:added:` + ' ' : kind === 'removed' ? $localize`:@@profile.history.srRemoved:removed:` + ' ' : '';
  }

  /** Validates the selected version in the engine, then saves it (undoable) and reloads the editor. */
  protected async restore(): Promise<void> {
    const text = this.versionText();
    if (text === null) return;
    this.restoring.set(true);
    this.restoreError.set(null);
    try {
      const v = await this.backend.validateProfile(text);
      if (!v.ok) {
        const reason = v.error ?? $localize`:@@profile.history.invalid:the profile is not valid`;
        this.restoreError.set(v.line ? $localize`:@@profile.history.invalidLine:line ${v.line}:line:: ${reason}:reason:` : reason);
        return;
      }
      await this.undo.save($localize`:Undo history entry|@@profile.history.undoLabel:profile restore`, text);
      await this.draft.load();
      this.store.setNotice($localize`:@@profile.history.restored:Restored the earlier profile version`);
    } catch (e) {
      this.restoreError.set(describeError(e));
    } finally {
      this.restoring.set(false);
    }
  }
}
