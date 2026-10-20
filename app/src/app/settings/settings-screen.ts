import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { DIALOG_REF } from '../ui/dialog.service';
import { AiSettingsExtras } from '../ai/settings/ai-settings-extras';
import { EngineSettingsStore } from './engine-settings.store';
import { AppearanceSettings } from './appearance-settings';
import { GeneralSettings } from './general-settings';
import { SettingFieldControl } from './setting-field';
import { SettingsFiles } from './settings-files';
import { StorageSettings } from './storage/storage-settings';
import { STORAGE_ID } from './storage/storage.types';
import { APPEARANCE_ID, fieldMatches, GENERAL_ID, GROUPS, statusLabel } from './settings-model';
import { SettingField, SettingsSection } from './settings.types';

/**
 * The Settings screen: navigation (General, then the engine's sections by group), search across
 * every field, a form generated from each section's schema, Apply/Cancel with an unsaved-changes
 * guard, a restart banner, and export/import. Opened in a dialog by `SettingsDialogService`.
 */
@Component({
  selector: 'imt-settings-screen',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [GeneralSettings, AppearanceSettings, SettingFieldControl, AiSettingsExtras, StorageSettings],
  host: { '(keydown.escape)': 'onEscape($event)' },
  template: `
    <div class="top">
      <input class="input search" type="search" placeholder="Search settings" aria-label="Search settings" i18n-placeholder="@@settings.screen.search.placeholder" i18n-aria-label="@@settings.screen.search.label" [value]="store.query()" (input)="store.setQuery($any($event.target).value)" />
      <label class="adv" i18n="@@settings.screen.showAdvancedToggle"><input type="checkbox" [checked]="store.showAdvanced()" (change)="store.setShowAdvanced($any($event.target).checked)" /> Show advanced</label>
    </div>

    @if (store.restartPending()) {
      <div class="banner restart" role="status">
        <span i18n="@@settings.screen.restartBanner">Some changes take effect after IMT restarts.</span>
        <button class="btn primary" type="button" (click)="store.restart()" i18n="@@settings.screen.restart">Restart</button>
      </div>
    }
    @if (store.confirmDiscard()) {
      <div class="banner warn" role="alertdialog" aria-label="Unapplied changes" i18n-aria-label="@@settings.screen.discard.label" aria-describedby="imt-settings-discard-text">
        <span id="imt-settings-discard-text" i18n="@@settings.screen.discard.text">You have changes that were not applied.</span>
        <button class="btn" type="button" (click)="discardAndClose()" i18n="@@settings.screen.discard.discard">Discard changes</button>
        <button class="btn primary" type="button" (click)="store.keepEditing()" i18n="@@settings.screen.discard.keepEditing">Keep editing</button>
      </div>
    }
    @if (store.notice(); as n) {
      <div class="banner ok" role="status"><span>{{ n }}</span><button class="btn ghost" type="button" (click)="store.dismissNotice()" i18n="@@settings.screen.dismiss">Dismiss</button></div>
    }
    @if (store.actionError(); as e) {
      <div class="banner err" role="alert"><span>{{ e }}</span><button class="btn ghost" type="button" (click)="store.dismissNotice()" i18n="@@settings.screen.dismiss">Dismiss</button></div>
    }

    <div class="main">
      <nav aria-label="Settings sections" i18n-aria-label="@@settings.screen.nav.label">
        <button class="nav" type="button" [attr.aria-current]="isSelected(general) ? 'page' : null" (click)="store.select(general)" i18n="Settings section|Name of the first settings section@@settings.screen.general">General</button>
        <button class="nav" type="button" [attr.aria-current]="isSelected(appearance) ? 'page' : null" (click)="store.select(appearance)" data-testid="settings-nav-appearance" i18n="Settings section|Theme, colour scheme and density@@settings.screen.appearance">Appearance</button>
        <button class="nav" type="button" [attr.aria-current]="isSelected(storage) ? 'page' : null" (click)="store.select(storage)" i18n="Settings section|What IMT keeps on disk@@settings.storage.nav">Storage</button>
        @for (g of navGroups(); track g.id) {
          <div class="group" role="presentation">{{ g.title }}</div>
          @for (s of g.sections; track s.id) {
            <button class="nav sub" type="button" [attr.aria-current]="isSelected(s.id) ? 'page' : null" (click)="store.select(s.id)">
              <span class="name">{{ s.title }}</span>
              @if (status(s); as st) {
                <span class="badge">{{ st }}</span>
              }
              @if (isDirty(s.id)) {
                <span class="dot" role="img" aria-label="unapplied changes" title="Unapplied changes" i18n-aria-label="@@settings.screen.dirtyDot.label" i18n-title="@@settings.screen.dirtyDot.title"></span>
              }
            </button>
          }
        }
      </nav>

      <section class="content" aria-label="Settings" i18n-aria-label="@@settings.screen.content.label">
        @if (store.loadError(); as e) {
          <div class="banner err" role="alert">
            <span i18n="@@settings.screen.loadError">Could not load the settings: {{ e }}</span>
            <button class="btn" type="button" (click)="store.load()" i18n="@@settings.screen.retry">Retry</button>
          </div>
        } @else if (!store.overview()) {
          <p class="muted" i18n="@@settings.screen.loading">Loading settings…</p>
        }
        @if (store.query().trim()) {
          @for (r of results(); track r.section.id) {
            <h3>{{ r.section.title }}</h3>
            @for (f of r.fields; track f.key) {
              <imt-setting-field
                [field]="f"
                [value]="store.valueOf(r.section.id, f.key)"
                [error]="store.errorFor(r.section.id, f.key)"
                [readOnly]="r.section.readOnly"
                (changed)="store.edit(r.section.id, f.key, $event)"
                (resetToDefault)="store.resetField(r.section.id, f.key)"
              />
            }
          } @empty {
            <p class="muted" i18n="@@settings.screen.noMatch">No settings match “{{ store.query() }}”.</p>
          }
        } @else if (isSelected(general)) {
          <h3 i18n="Settings section|Name of the first settings section@@settings.screen.general">General</h3>
          <p class="desc" i18n="@@settings.screen.general.desc">Display preferences of this window. They apply immediately.</p>
          <imt-general-settings (layoutReset)="close()" />
        } @else if (isSelected(appearance)) {
          <h3 i18n="Settings section|Theme, colour scheme and density@@settings.screen.appearance">Appearance</h3>
          <p class="desc" i18n="@@settings.screen.appearance.desc">Theme, colour scheme and density of every window. They apply immediately.</p>
          <imt-appearance-settings />
        } @else if (isSelected(storage)) {
          <imt-storage-settings />
        } @else if (store.current(); as s) {
          <div class="section-head">
            <h3>{{ s.title }}</h3>
            @if (status(s); as st) {
              <span class="badge">{{ st }}</span>
            }
            <span class="spacer"></span>
            @if (s.fields.length) {
              <button class="btn" type="button" [disabled]="s.readOnly || store.busy()" (click)="store.resetSection(s.id)" i18n="@@settings.screen.resetSection">Reset section</button>
            }
          </div>
          @if (s.description) {
            <p class="desc">{{ s.description }}</p>
          }
          @if (s.note) {
            <p class="note" role="note">{{ s.note }}</p>
          }
          @if (s.id === 'ai') {
            @defer {
              <imt-ai-settings-extras />
            }
          }
          @for (e of sectionErrors(s.id); track $index) {
            <p class="error" role="alert">{{ e }}</p>
          }
          @for (f of visibleFields(s); track f.key) {
            <imt-setting-field
              [field]="f"
              [value]="store.valueOf(s.id, f.key)"
              [error]="store.errorFor(s.id, f.key)"
              [readOnly]="s.readOnly"
              (changed)="store.edit(s.id, f.key, $event)"
              (resetToDefault)="store.resetField(s.id, f.key)"
            />
          }
          @if (hiddenAdvanced(s); as n) {
            <p class="muted" i18n="@@settings.screen.hiddenAdvanced">{n, plural, =1 {{{ n }} advanced setting hidden.} other {{{ n }} advanced settings hidden.}} <button class="link" type="button" (click)="store.setShowAdvanced(true)">Show advanced</button></p>
          }
          @if (!s.fields.length && s.status !== 'notInstalled') {
            <p class="muted" i18n="@@settings.screen.emptySection">This section has no settings.</p>
          }
        }
      </section>
    </div>

    <div class="footer">
      <button class="btn" type="button" [disabled]="store.busy()" (click)="exportSettings()" i18n="@@settings.screen.export">Export…</button>
      <button class="btn" type="button" [disabled]="store.busy()" (click)="importSettings()" i18n="@@settings.screen.import">Import…</button>
      <span class="spacer"></span>
      @if (store.dirty()) {
        <span class="muted" role="status" i18n="@@settings.screen.unappliedIn">{store.dirtySections().length, plural, =1 {Unapplied changes in {{ store.dirtySections().length }} section} other {Unapplied changes in {{ store.dirtySections().length }} sections}}</span>
      }
      <button class="btn" type="button" (click)="cancel()">{{ store.dirty() ? cancelLabel : closeLabel }}</button>
      <button class="btn primary" type="button" [disabled]="!store.dirty() || store.busy()" (click)="store.apply()" i18n="@@settings.screen.apply">Apply</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; height: min(640px, calc(100vh - 170px)); min-height: 0; }
    .top { display: flex; align-items: center; gap: 12px; }
    .search { flex: 1; }
    .adv { display: inline-flex; align-items: center; gap: 6px; color: var(--text-2); white-space: nowrap; }
    .banner { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text-2); }
    .banner > span { flex: 1; }
    .banner.restart { border-color: var(--accent); background: var(--accent-soft); color: var(--text); }
    .banner.warn { border-color: var(--warn-text); }
    .banner.err { border-color: var(--danger-text); color: var(--danger-text); }
    .banner.ok { border-color: var(--ok); }
    .main { flex: 1; min-height: 0; display: flex; gap: 16px; }
    nav { flex: none; width: 200px; overflow: auto; display: flex; flex-direction: column; gap: 2px; border-inline-end: 1px solid var(--border); padding-inline-end: 8px; }
    .nav { display: flex; align-items: center; gap: 6px; text-align: start; padding: 6px 8px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; }
    .nav.sub { padding-inline-start: 16px; }
    .nav:hover { background: var(--bg-hover); }
    .nav[aria-current='page'] { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .nav .name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .group { margin-top: 10px; padding: 0 8px 2px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); }
    .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); flex: none; }
    .content { flex: 1; min-width: 0; overflow: auto; padding-inline-end: 4px; }
    h3 { margin: 0 0 4px; font-size: var(--fs-lg); font-weight: 600; }
    .content > h3:not(:first-child) { margin-top: 16px; }
    .section-head { display: flex; align-items: center; gap: 8px; }
    .spacer { flex: 1; }
    .desc { margin: 4px 0 8px; color: var(--muted); line-height: 1.45; }
    .note { margin: 4px 0 8px; padding: 6px 8px; border-inline-start: 2px solid var(--warn-text); color: var(--text-2); }
    .error { margin: 4px 0; color: var(--danger-text); }
    .link { border: 0; background: none; padding: 0; color: var(--text); cursor: pointer; text-decoration: underline; }
    .footer { display: flex; align-items: center; gap: 8px; padding-top: 10px; border-top: 1px solid var(--border); }
  `,
})
export class SettingsScreen {
  protected readonly store = inject(EngineSettingsStore);
  private readonly files = inject(SettingsFiles);
  private readonly ref = inject(DIALOG_REF, { optional: true });
  protected readonly general = GENERAL_ID;
  protected readonly storage = STORAGE_ID;
  protected readonly appearance = APPEARANCE_ID;
  protected readonly cancelLabel = $localize`:@@settings.screen.cancel:Cancel`;
  protected readonly closeLabel = $localize`:Button that closes the settings dialog@@settings.screen.close:Close`;

  protected readonly navGroups = computed(() =>
    GROUPS.map((g) => ({ ...g, sections: this.store.sections().filter((s) => s.group === g.id) })).filter((g) => g.sections.length > 0),
  );

  /** Search hits across every section (advanced settings included). */
  protected readonly results = computed(() => {
    const q = this.store.query().trim();
    return this.store
      .sections()
      .map((section) => ({ section, fields: section.fields.filter((f) => fieldMatches(section, f, q)) }))
      .filter((r) => r.fields.length > 0);
  });

  constructor() {
    void this.store.load();
  }

  protected isSelected(id: string): boolean {
    return this.store.selected() === id;
  }

  protected isDirty(id: string): boolean {
    return this.store.dirtySections().includes(id);
  }

  protected status(s: SettingsSection): string | null {
    return statusLabel(s);
  }

  /** Advanced fields are shown when asked, or when they hold an edit or an error. */
  protected visibleFields(s: SettingsSection): SettingField[] {
    if (this.store.showAdvanced()) return s.fields;
    return s.fields.filter((f) => !f.advanced || this.store.drafts()[s.id]?.[f.key] !== undefined || !!this.store.errorFor(s.id, f.key));
  }

  protected hiddenAdvanced(s: SettingsSection): number {
    return s.fields.length - this.visibleFields(s).length;
  }

  /** Errors about the whole section (e.g. the plugin refused the values). */
  protected sectionErrors(id: string): string[] {
    return (this.store.errors()[id] ?? []).filter((e) => !e.key || !this.store.sections().find((s) => s.id === id)?.fields.some((f) => f.key === e.key)).map((e) => e.message);
  }

  /** Escape with unapplied edits asks first instead of letting the dialog close. */
  protected onEscape(e: Event): void {
    if (this.store.canClose()) return;
    e.preventDefault();
    e.stopPropagation();
  }

  protected cancel(): void {
    if (this.store.canClose()) this.close();
  }

  protected discardAndClose(): void {
    this.store.discard();
    this.close();
  }

  protected close(): void {
    this.ref?.close();
  }

  protected async exportSettings(): Promise<void> {
    const path = await this.files.pickExport();
    if (path) await this.store.exportTo(path);
  }

  protected async importSettings(): Promise<void> {
    const path = await this.files.pickImport();
    if (path) await this.store.importFrom(path);
  }
}
