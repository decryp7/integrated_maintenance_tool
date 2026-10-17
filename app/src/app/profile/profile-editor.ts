import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { Icon } from '../shared/icon';
import { ProfileDraft } from './profile-draft';
import { ProfileForm } from './profile-form';
import { TomlEditor } from './toml-editor';
import { ProfileHistory } from './history/profile-history';
import { TemplatesService } from '../templates/templates.service';

@Component({
  selector: 'imt-profile-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ProfileForm, TomlEditor, Icon, ProfileHistory, AiButton],
  providers: [ProfileDraft],
  host: { '(keydown)': 'onKey($event)' },
  template: `
    <div class="bar">
      <span class="section-title">Parsing profile</span>
      <span class="name">{{ draft.model().name ?? draft.doc()?.name ?? '…' }}</span>
      <span class="mono faint path ellipsis">{{ draft.importedFrom() ?? draft.doc()?.path ?? 'built-in default' }}</span>
      @if (draft.dirty()) { <span class="unsaved">● Unsaved</span> }
      @if (draft.problem(); as p) {
        <span class="bad"><imt-icon name="alert" [size]="13" /> {{ p.line ? 'Line ' + p.line + ': ' : '' }}{{ p.message }}</span>
      } @else if (draft.validation()?.ok) {
        <span class="good"><imt-icon name="check" [size]="13" /> Valid</span>
      }
      <span class="spacer"></span>
      <div class="seg" role="tablist">
        <button role="tab" class="btn" [class.active]="mode() === 'form'" [attr.aria-selected]="mode() === 'form'" (click)="mode.set('form')">Form</button>
        <button role="tab" class="btn" [class.active]="mode() === 'toml'" [attr.aria-selected]="mode() === 'toml'" (click)="mode.set('toml')">TOML</button>
        <button role="tab" class="btn" [class.active]="mode() === 'history'" [attr.aria-selected]="mode() === 'history'" (click)="mode.set('history')">History</button>
      </div>
      <button class="btn ghost" [disabled]="draft.dirty()" [title]="draft.dirty() ? 'Save or revert your edits first' : 'Start from a ready-made profile for a common log format'" (click)="fromTemplate()">New from template…</button>
      <imt-ai-button label="Propose…" [disabled]="draft.dirty()" [hint]="draft.dirty() ? 'Save or revert your edits first' : 'Let the local model propose pipeline steps, markers or extractors from the file of the selected event'" testId="ai-profile-propose" (pressed)="proposeWithAi()" />
      <button class="btn ghost" (click)="draft.importFile()">Import…</button>
      <button class="btn ghost" (click)="draft.exportFile()">Export…</button>
      <button class="btn" [disabled]="!draft.dirty()" (click)="draft.revert()">Revert</button>
      <button class="btn primary" [disabled]="!draft.dirty() || !!draft.problem() || draft.saving()" (click)="draft.save()" title="Apply and re-import (Ctrl/⌘+S)">
        {{ draft.saving() ? 'Applying…' : 'Save & apply' }}
      </button>
    </div>
    @if (draft.stale()) {
      <div class="stale" role="alert">
        <imt-icon name="alert" [size]="13" /> The active profile changed (undo or restore). Your edits are based on the previous version.
        <button type="button" class="btn" (click)="draft.revert()">Discard my edits</button>
      </div>
    }
    @if (mode() === 'history') {
      <imt-profile-history />
    } @else if (mode() === 'form') {
      @if (draft.parseError(); as e) {
        <div class="empty-state">
          <div>The TOML has a syntax error, so the form can't show it.</div>
          <div class="faint">{{ e.line ? 'Line ' + e.line + ': ' : '' }}{{ e.message }}</div>
          <button class="btn" (click)="mode.set('toml')">Fix in TOML view</button>
        </div>
      } @else {
        <imt-profile-form />
      }
    } @else {
      <div class="note faint">Comments are kept while you edit here; editing in the form rewrites the file without comments.</div>
      <imt-toml-editor />
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 8px 14px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .name { font-weight: 600; }
    .path { font-size: 11px; max-width: 280px; }
    .unsaved { color: var(--warn-text); font-size: 12px; }
    .bad { display: inline-flex; align-items: center; gap: 4px; color: var(--danger-text); font-size: 12px; max-width: 420px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .good { display: inline-flex; align-items: center; gap: 4px; color: var(--ok); font-size: 12px; }
    .spacer { flex: 1; }
    .seg { display: flex; gap: 2px; }
    .stale { flex: none; display: flex; align-items: center; gap: 8px; padding: 6px 14px; font-size: 12px; color: var(--warn-text); border-bottom: 1px solid var(--border); background: var(--bg-2); }
    .note { flex: none; padding: 6px 14px; font-size: 11px; border-bottom: 1px solid var(--border); }
  `,
})
export class ProfileEditor {
  protected readonly draft = inject(ProfileDraft);
  protected readonly mode = signal<'form' | 'toml' | 'history'>('form');
  private readonly templates = inject(TemplatesService);
  private readonly ai = inject(AiLauncher);
  private readonly workspace = inject(WorkspaceStore);

  constructor() {
    void this.draft.load();
  }

  /** "Propose pipeline/markers/extractors" with the local AI model, on the selected event's file. */
  protected proposeWithAi(): void {
    void this.ai.propose('pipeline', { recordId: this.workspace.selected()?.id ?? null }, '', { chooseKind: true });
  }

  /** Opens the templates gallery; a template that was applied is loaded into the editor. */
  protected async fromTemplate(): Promise<void> {
    if (await this.templates.open()) await this.draft.load();
  }

  protected onKey(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      if (this.draft.dirty() && !this.draft.problem()) void this.draft.save();
    }
  }
}
