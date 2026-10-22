import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { DIALOG_REF } from '../ui/dialog.service';
import { AiSettingsExtras } from '../ai/settings/ai-settings-extras';
import { PerformanceExtras } from './performance/performance-extras';
import { LiveTailSettings } from '../live/live-tail-settings';
import { BrowserRuntimeSettings } from '../browser/browser-runtime-settings';
import { UpdatesSettings } from '../updates/updates-settings';
import { KnowledgeSettings } from '../ai/knowledge/knowledge-settings';
import { EngineSettingsStore } from './engine-settings.store';
import { AppearanceSettings } from './appearance-settings';
import { GeneralSettings } from './general-settings';
import { SettingFieldControl } from './setting-field';
import { SettingsFiles } from './settings-files';
import { StorageSettings } from './storage/storage-settings';
import { MANAGE_PARSERS_COMMAND, PARSERS_ID } from './parsers/parsers.types';
import { CommandRegistry } from '../extensions/command-registry';
import { KeyboardSettings } from './keyboard/keyboard-settings';
import { KEYBOARD_ID } from './keyboard/keyboard.types';
import { STORAGE_ID } from './storage/storage.types';
import { ParsersStore } from './parsers/parsers.store';
import { VideoSettings } from '../video/video-settings';
import { VIDEO_SETTINGS_ID } from '../video/video-settings.types';
import { searchParsers } from './parsers/parser-search';
import { APPEARANCE_ID, fieldMatches, GENERAL_ID, LIVE_TAIL_SECTION, navLayout, statusLabel, UPDATES_ID } from './settings-model';
import { plural } from '../i18n/locale';
import { SettingField, SettingsSection } from './settings.types';

/**
 * The Settings screen: grouped navigation (Application, Data, AI, Plugins, Diagnostics), search across
 * every field and a form generated from each section's schema. Changes apply by themselves (with Undo);
 * only edits the engine refuses stay unsaved and are asked about on close. Settings that need a restart
 * are marked inline and collected in one bar. Opened in a dialog by `SettingsDialogService`.
 */
@Component({
  selector: 'debrief-settings-screen',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [GeneralSettings, AppearanceSettings, SettingFieldControl, AiSettingsExtras, KnowledgeSettings, StorageSettings, PerformanceExtras, LiveTailSettings, BrowserRuntimeSettings, UpdatesSettings, KeyboardSettings, VideoSettings],
  host: { '(keydown.escape)': 'onEscape($event)' },
  template: `
    <div class="top">
      <input class="input search" type="search" placeholder="Search settings" aria-label="Search settings" i18n-placeholder="@@settings.screen.search.placeholder" i18n-aria-label="@@settings.screen.search.label" [value]="store.query()" (input)="store.setQuery($any($event.target).value)" />
      <label class="adv" i18n="@@settings.screen.showAdvancedToggle"><input type="checkbox" [checked]="store.showAdvanced()" (change)="store.setShowAdvanced($any($event.target).checked)" /> Show advanced</label>
    </div>

    @if (store.restartPending()) {
      <div class="banner restart" role="status" data-testid="settings-restart-bar">
        <span>{{ restartText() }}</span>
        <button class="btn primary" type="button" (click)="store.restart()" i18n="@@settings.screen.restartNow">Restart now</button>
      </div>
    }
    @if (store.confirmDiscard()) {
      <div class="banner warn" role="alertdialog" aria-label="Unsaved changes" i18n-aria-label="@@settings.screen.discard.label" aria-describedby="debrief-settings-discard-text">
        <span id="debrief-settings-discard-text" i18n="@@settings.screen.discard.text">Some changes were refused and are not saved.</span>
        <button class="btn" type="button" (click)="discardAndClose()" i18n="@@settings.screen.discard.discard">Discard changes</button>
        <button class="btn primary" type="button" (click)="store.keepEditing()" i18n="@@settings.screen.discard.keepEditing">Keep editing</button>
      </div>
    }
    @if (store.notice(); as n) {
      <div class="banner ok" role="status" data-testid="settings-notice">
        <span>{{ n }}</span>
        @if (store.undo()) {
          <button class="btn" type="button" data-testid="settings-undo" (click)="store.undoLast()" i18n="@@settings.screen.undo">Undo</button>
        }
        <button class="btn ghost" type="button" (click)="store.dismissNotice()" i18n="@@settings.screen.dismiss">Dismiss</button>
      </div>
    }
    @if (store.actionError(); as e) {
      <div class="banner err" role="alert"><span>{{ e }}</span><button class="btn ghost" type="button" (click)="store.dismissNotice()" i18n="@@settings.screen.dismiss">Dismiss</button></div>
    }

    <div class="main">
      <nav aria-label="Settings sections" i18n-aria-label="@@settings.screen.nav.label">
        @for (g of navGroups(); track g.id) {
          <div class="group" role="presentation">{{ g.title }}</div>
          @for (i of g.items; track i.id) {
            <button class="nav sub" type="button" [attr.aria-current]="isSelected(i.id) ? 'page' : null" [attr.data-testid]="'settings-nav-' + navKey(i.id)" (click)="store.select(i.id)">
              <span class="name">{{ i.title }}</span>
              @if (i.section && status(i.section); as st) {
                <span class="badge">{{ st }}</span>
              }
              @if (isDirty(i.id)) {
                <span class="dot" role="img" aria-label="unsaved changes" title="Unsaved changes" i18n-aria-label="@@settings.screen.dirtyDot.label" i18n-title="@@settings.screen.dirtyDot.title"></span>
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
          @if (parserHits(); as ph) {
            <h3 i18n="Settings section|The log formats Debrief reads@@settings.parsers.nav">Parsers</h3>
            @if (ph.parsers.length) {
              <ul class="parser-hits" aria-label="Matching parsers" i18n-aria-label="@@settings.screen.search.parserHits">
                @for (p of ph.parsers; track p.id) {
                  <li data-testid="settings-search-parser">{{ p.name }} <span class="muted">{{ p.id }} · {{ p.category }}</span></li>
                }
              </ul>
            }
            <button class="btn" type="button" data-testid="settings-search-open-parsers" (click)="openParsers()" i18n="@@settings.screen.search.openParsers">Open Parsers</button>
          }
          @for (r of results(); track r.section.id) {
            <h3>{{ titleOf(r.section) }}</h3>
            @for (f of r.fields; track f.key) {
              <debrief-setting-field
                [field]="f"
                [value]="store.valueOf(r.section.id, f.key)"
                [error]="store.errorFor(r.section.id, f.key)"
                [readOnly]="r.section.readOnly"
                (changed)="store.edit(r.section.id, f.key, $event)"
                (resetToDefault)="store.resetField(r.section.id, f.key)"
              />
            }
          } @empty {
            @if (!parserHits()) {
              <p class="muted" i18n="@@settings.screen.noMatch">No settings match “{{ store.query() }}”.</p>
            }
          }
        } @else if (isSelected(general)) {
          <h3 i18n="Settings section|Name of the first settings section@@settings.screen.general">General</h3>
          <p class="desc" i18n="@@settings.screen.general.desc">Display preferences of this window. They apply immediately.</p>
          <debrief-general-settings (layoutReset)="close()" />
        } @else if (isSelected(appearance)) {
          <h3 i18n="Settings section|Theme, colour scheme and density@@settings.screen.appearance">Appearance</h3>
          <p class="desc" i18n="@@settings.screen.appearance.desc">Theme, colour scheme and density of every window. They apply immediately.</p>
          <debrief-appearance-settings />
        } @else if (isSelected(updates)) {
          <debrief-updates-settings />
        } @else if (isSelected(storage)) {
          <debrief-storage-settings />
        } @else if (isSelected(parsers)) {
          <h3 i18n="Settings section|The log formats Debrief reads@@settings.parsers.title">Parsers</h3>
          <p class="desc" i18n="@@settings.parsers.link.desc">A parser describes one log format and is one TOML file you can share. The parser library, with its editor and a live preview against your open files, is a view of its own.</p>
          <button type="button" class="btn primary" data-testid="settings-open-parsers-view" (click)="openParsersView()" i18n="@@settings.parsers.link.open">Open the Parsers view</button>
        } @else if (isSelected(keyboard)) {
          <h3 i18n="Settings section|Key bindings of the commands@@settings.keyboard.nav">Keyboard shortcuts</h3>
          <debrief-keyboard-settings />
        } @else if (isSelected(video)) {
          <debrief-video-settings />
        } @else if (store.current(); as s) {
          <div class="section-head">
            <h3>{{ titleOf(s) }}</h3>
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
          @if (s.id === 'performance' && !s.readOnly) {
            <debrief-performance-extras />
          }
          @if (s.id === 'ai') {
            @defer {
              <debrief-ai-settings-extras />
              <debrief-knowledge-settings />
            }
          }
          @for (e of sectionErrors(s.id); track $index) {
            <p class="error" role="alert">{{ e }}</p>
          }
          @for (f of visibleFields(s); track f.key) {
            <debrief-setting-field
              [field]="f"
              [value]="store.valueOf(s.id, f.key)"
              [error]="store.errorFor(s.id, f.key)"
              [readOnly]="s.readOnly"
              (changed)="store.edit(s.id, f.key, $event)"
              (resetToDefault)="store.resetField(s.id, f.key)"
            />
          }
          @if (s.id === liveTail) {
            <debrief-live-tail-settings />
          }
          @if (s.id === 'browser') {
            <debrief-browser-runtime-settings />
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
      <button class="btn" type="button" [disabled]="store.busy()" (click)="exportSettings()" i18n="@@settings.screen.exportSettings">Export settings…</button>
      <button class="btn" type="button" [disabled]="store.busy()" (click)="importSettings()" i18n="@@settings.screen.importSettings">Import settings…</button>
      <span class="spacer"></span>
      @if (store.rejectedSections().length) {
        <span class="muted" role="status" i18n="@@settings.screen.unsavedIn">{store.rejectedSections().length, plural, =1 {Not saved in {{ store.rejectedSections().length }} section} other {Not saved in {{ store.rejectedSections().length }} sections}}</span>
      }
      <button class="btn" type="button" (click)="cancel()" i18n="Button that closes the settings dialog@@settings.screen.close">Close</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; height: min(640px, calc(100vh - 170px)); min-height: 0; }
    .top { display: flex; align-items: center; gap: 12px; }
    .search { flex: 1; }
    .parser-hits { margin: 0 0 8px; padding-left: 18px; }
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
  private readonly parserLibrary = inject(ParsersStore);
  private readonly parsersTitle = $localize`:Settings section|The log formats Debrief reads@@settings.parsers.nav:Parsers`;
  private readonly files = inject(SettingsFiles);
  private readonly ref = inject(DIALOG_REF, { optional: true });
  protected readonly general = GENERAL_ID;
  protected readonly updates = UPDATES_ID;
  protected readonly liveTail = LIVE_TAIL_SECTION;
  protected readonly storage = STORAGE_ID;
  private readonly commands = inject(CommandRegistry);
  protected readonly parsers = PARSERS_ID;
  protected readonly keyboard = KEYBOARD_ID;
  protected readonly video = VIDEO_SETTINGS_ID;
  protected readonly appearance = APPEARANCE_ID;

  protected readonly navGroups = computed(() => navLayout(this.store.sections()));

  /** The restart bar: how many restart-only settings were changed, or the general reminder. */
  protected readonly restartText = computed(() => {
    const n = this.store.restartChanges().length;
    return n
      ? plural(n, {
          one: $localize`:Restart bar in Settings@@settings.screen.restartBar.one:${n}:count: change takes effect after Debrief restarts.`,
          other: $localize`:Restart bar in Settings@@settings.screen.restartBar.other:${n}:count: changes take effect after Debrief restarts.`,
        })
      : $localize`:@@settings.screen.restartBanner:Some changes take effect after Debrief restarts.`;
  });

  /** Search hits across every section (advanced settings included). */
  protected readonly results = computed(() => {
    const q = this.store.query().trim();
    return this.store
      .sections()
      .map((section) => ({ section, fields: section.fields.filter((f) => fieldMatches(section, f, q)) }))
      .filter((r) => r.fields.length > 0);
  });

  /** Search hits in the Parsers section (the section and the library's parsers). */
  protected readonly parserHits = computed(() => searchParsers(this.parserLibrary.parsers(), this.store.query(), this.parsersTitle));

  constructor() {
    void this.store.load();
    // The search covers the parser library too, before the Parsers section was ever opened.
    if (!this.parserLibrary.library() && !this.parserLibrary.loading()) void this.parserLibrary.load();
  }

  /** Opens the Parsers view and closes Settings. */
  protected openParsersView(): void {
    void this.commands.run(MANAGE_PARSERS_COMMAND);
    this.close();
  }

  /** Leaves the search for the Parsers section. */
  protected openParsers(): void {
    this.store.setQuery('');
    this.store.select(PARSERS_ID);
  }

  protected isSelected(id: string): boolean {
    return this.store.selected() === id;
  }

  /** A section with edits the engine refused. */
  protected isDirty(id: string): boolean {
    return this.store.rejectedSections().includes(id);
  }

  /** The id used in test ids: engine ids as they are, UI pages without the leading `@`. */
  protected navKey(id: string): string {
    return id.replace(/^@/, '');
  }

  /** The section's title; the live tail delay's section is the Live tail page. */
  protected titleOf(s: SettingsSection): string {
    return this.navGroups().flatMap((g) => g.items).find((i) => i.id === s.id)?.title ?? s.title;
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
