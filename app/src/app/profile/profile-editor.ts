import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import { CommandRegistry } from '../extensions/command-registry';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { Icon } from '../shared/icon';
import { formatKeybinding } from '../extensions/keybinding';
import { THEMED_CONTROLS } from '../settings/themed-controls';
import { ProfileDraft } from './profile-draft';
import { ProfileForm } from './profile-form';
import { TomlEditor } from './toml-editor';
import { ProfileHistory } from './history/profile-history';
import { SettingsDialogService } from '../settings/settings-dialog';
import { MANAGE_PARSERS_COMMAND } from '../settings/parsers/parsers.types';

@Component({
  selector: 'debrief-profile-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkMenu, CdkMenuItem, CdkMenuTrigger, ProfileForm, TomlEditor, Icon, ProfileHistory, AiButton],
  providers: [ProfileDraft],
  host: { '(keydown)': 'onKey($event)' },
  template: `
    <div class="bar">
      <!-- The profile's name and state arrive after the view opens: they fill a slot of fixed basis and truncate, so the
           controls beside them never move under the pointer when they do (ADR 0061). -->
      <div class="ident">
        <span class="name ellipsis" [title]="draft.model().name ?? draft.doc()?.name ?? ''">{{ draft.model().name ?? draft.doc()?.name ?? '…' }}</span>
        <span class="mono faint path ellipsis" [title]="draft.importedFrom() ?? draft.doc()?.path ?? builtInDefault">{{ draft.importedFrom() ?? draft.doc()?.path ?? builtInDefault }}</span>
        @if (draft.dirty()) { <span class="unsaved" i18n="@@profile.editor.unsaved">● Unsaved</span> }
        @if (draft.problem(); as p) {
          <span class="bad"><debrief-icon name="alert" [size]="13" /> {{ problemText(p) }}</span>
        } @else if (draft.validation()?.ok) {
          <span class="good" i18n="@@profile.editor.valid"><debrief-icon name="check" [size]="13" /> Valid</span>
        }
      </div>
      <div class="seg" role="tablist">
        <button role="tab" class="btn" [class.active]="mode() === 'form'" [attr.aria-selected]="mode() === 'form'" (click)="mode.set('form')" i18n="@@profile.editor.tab.form">Form</button>
        <button role="tab" class="btn" [class.active]="mode() === 'toml'" [attr.aria-selected]="mode() === 'toml'" (click)="mode.set('toml')">TOML</button>
        <button role="tab" class="btn" [class.active]="mode() === 'history'" [attr.aria-selected]="mode() === 'history'" (click)="mode.set('history')" i18n="@@profile.editor.tab.history">History</button>
      </div>
      <button class="btn ghost" data-testid="profile-parsers" title="File formats are parsers: Settings ▸ Parsers" i18n-title="@@profile.editor.parsersHint" (click)="openParsers()" i18n="@@profile.editor.parsers">Parsers…</button>
      <debrief-ai-button label="Propose…" i18n-label="@@profile.editor.propose" [disabled]="draft.dirty()" [hint]="proposeHint()" testId="ai-profile-propose" (pressed)="proposeWithAi()" />
      <button type="button" class="btn ghost icon-btn" data-testid="profile-more" aria-label="More profile actions" i18n-aria-label="@@profile.editor.more" title="More actions" i18n-title="@@profile.editor.more.title" [cdkMenuTriggerFor]="moreMenu"><debrief-icon name="more" [size]="14" /></button>
      <button class="btn primary" [disabled]="!draft.dirty() || !!draft.problem() || draft.saving()" (click)="draft.save()" [title]="saveTitle">
        @if (draft.saving()) { <ng-container i18n="@@profile.editor.applying">Applying…</ng-container> } @else { <ng-container i18n="@@profile.editor.save">Save &amp; apply</ng-container> }
      </button>
    </div>
    <ng-template #moreMenu>
      <div cdkMenu class="menu" aria-label="Profile actions" i18n-aria-label="@@profile.editor.moreMenu">
        <button type="button" cdkMenuItem class="mi" (cdkMenuItemTriggered)="draft.importFile()" i18n="@@profile.editor.import">Import…</button>
        <button type="button" cdkMenuItem class="mi" (cdkMenuItemTriggered)="draft.exportFile()" i18n="@@profile.editor.export">Export…</button>
        <button type="button" cdkMenuItem class="mi" [disabled]="!draft.dirty()" (cdkMenuItemTriggered)="draft.revert()" i18n="@@profile.editor.revert">Revert</button>
      </div>
    </ng-template>
    @if (draft.stale()) {
      <div class="stale" role="alert">
        <debrief-icon name="alert" [size]="13" />&ngsp;<ng-container i18n="@@profile.editor.stale">The active profile changed (undo or restore). Your edits are based on the previous version.</ng-container>
        <button type="button" class="btn" (click)="draft.revert()" i18n="@@profile.editor.discard">Discard my edits</button>
      </div>
    }
    @if (mode() === 'history') {
      <debrief-profile-history />
    } @else if (mode() === 'form') {
      @if (draft.parseError(); as e) {
        <div class="empty-state">
          <div i18n="@@profile.editor.syntaxError">The TOML has a syntax error, so the form can't show it.</div>
          <div class="faint">{{ problemText(e) }}</div>
          <button class="btn" (click)="mode.set('toml')" i18n="@@profile.editor.fixToml">Fix in TOML view</button>
        </div>
      } @else {
        <debrief-profile-form />
      }
    } @else {
      <div class="note faint" i18n="@@profile.editor.note">Comments are kept while you edit here; editing in the form rewrites the file without comments.</div>
      <debrief-toml-editor />
    }
  `,
  styles: [
    THEMED_CONTROLS,
    `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 8px 14px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    /* Basis 0 and a fixed minimum: where the controls sit (and whether they wrap) depends on the panel's width only, never on the name, path or state shown here. */
    .ident { flex: 1 1 0; min-width: 160px; display: flex; align-items: center; gap: 10px; overflow: hidden; }
    .ident > * { flex: 0 1 auto; min-width: 0; }
    .ident > .unsaved, .ident > .good { flex: none; }
    .name { font-weight: 600; }
    .path { font-size: var(--fs-xs); max-width: 280px; }
    .unsaved { color: var(--warn-text); font-size: var(--fs-sm); white-space: nowrap; }
    .bad { display: inline-flex; align-items: center; gap: 4px; color: var(--danger-text); font-size: var(--fs-sm); max-width: 420px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .good { display: inline-flex; align-items: center; gap: 4px; color: var(--ok); font-size: var(--fs-sm); white-space: nowrap; }
    .seg { display: flex; gap: 2px; }
    .stale { flex: none; display: flex; align-items: center; gap: 8px; padding: 6px 14px; font-size: var(--fs-sm); color: var(--warn-text); border-bottom: 1px solid var(--border); background: var(--bg-2); }
    .note { flex: none; padding: 6px 14px; font-size: var(--fs-xs); border-bottom: 1px solid var(--border); }
    .menu { min-width: 160px; display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .mi { display: flex; align-items: center; min-height: 28px; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: start; font: inherit; }
    .mi:hover, .mi:focus { background: var(--bg-active); color: var(--text); outline: none; }
    .mi:disabled { opacity: 0.5; cursor: default; }
  `,
  ],
})
export class ProfileEditor {
  protected readonly draft = inject(ProfileDraft);
  protected readonly saveTitle = $localize`:@@profile.editor.saveTitleKey:Apply and re-import (${formatKeybinding('Mod+S')}:key:)`;
  protected readonly mode = signal<'form' | 'toml' | 'history'>('form');
  private readonly commands = inject(CommandRegistry);
  private readonly settings = inject(SettingsDialogService);
  private readonly ai = inject(AiLauncher);
  private readonly workspace = inject(WorkspaceStore);

  protected readonly builtInDefault = $localize`:@@profile.editor.builtInDefault:built-in default`;
  protected readonly proposeHint = computed(() =>
    this.draft.dirty()
      ? $localize`:@@profile.editor.saveFirst:Save or revert your edits first`
      : $localize`:@@profile.editor.proposeHint:Let the local model propose pipeline steps, markers or extractors from the file of the selected event`,
  );

  constructor() {
    void this.draft.load();
  }

  protected problemText(p: { line?: number | null; message: string }): string {
    return p.line ? $localize`:@@profile.editor.lineMessage:Line ${p.line}:line:: ${p.message}:message:` : p.message;
  }

  /** "Propose pipeline/markers/extractors" with the local AI model, on the selected event's file. */
  protected proposeWithAi(): void {
    void this.ai.propose('pipeline', { recordId: this.workspace.selected()?.id ?? null }, '', { chooseKind: true });
  }

  /** How files are read is the parsers' business (ADR 0026): the Parsers view. */
  protected openParsers(): void {
    void this.commands.run(MANAGE_PARSERS_COMMAND);
  }

  protected onKey(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      if (this.draft.dirty() && !this.draft.problem()) void this.draft.save();
    }
  }
}
