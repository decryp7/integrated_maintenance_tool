import { ChangeDetectionStrategy, Component, effect, inject, OnInit, signal, untracked } from '@angular/core';
import { Platform } from '../../core/platform';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { formatBytes } from '../ai-format';
import { toAiError } from '../ai-backend';
import { KnowledgeInfo, KnowledgeScope } from '../ai.types';
import { openKnowledgeEditor } from './knowledge-editor';
import { KnowledgePersistence } from './knowledge-persistence';
import { KnowledgeStore } from './knowledge.store';

const MARKDOWN = [{ name: 'Markdown', extensions: ['md', 'markdown', 'txt'] }];

/**
 * Settings ▸ AI ▸ Knowledge: the knowledge files of this project (saved with it) and of every project (IMT's
 * configuration folder). Add (write or import), edit with a preview, enable or disable, export and remove (asked first).
 */
@Component({
  selector: 'imt-knowledge-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <h4 i18n="@@ai.knowledge.title">Knowledge</h4>
    <p class="desc" i18n="@@ai.knowledge.desc">
      Markdown notes the AI reads with every question: the names of your products and services, what log sources mean, known errors and their
      fixes, facts about your environments. Project files are saved with the project, so a shared project carries them.
    </p>
    @if (store.loadError(); as e) {
      <div class="banner err" role="alert">
        <span i18n="@@ai.knowledge.loadError">The knowledge files could not be listed: {{ e }}</span>
        <button type="button" class="btn" (click)="store.load()" i18n="@@ai.knowledge.retry">Retry</button>
      </div>
    }
    @for (w of store.warnings(); track $index) {
      <p class="note" role="note">{{ w }}</p>
    }
    @if (persistence.warning(); as w) {
      <p class="note" role="note">{{ w }}</p>
    }
    @if (store.actionError(); as e) {
      <div class="banner err" role="alert" data-testid="knowledge-error">
        <span>{{ e }}</span>
        <button type="button" class="btn ghost" (click)="store.dismissError()" i18n="@@ai.knowledge.dismiss">Dismiss</button>
      </div>
    }
    @if (notice(); as n) {
      <p class="muted" role="status" data-testid="knowledge-notice">{{ n }}</p>
    }
    @for (s of scopes; track s.scope) {
      <section [attr.aria-label]="s.title" [attr.data-testid]="'knowledge-' + s.scope">
        <div class="head">
          <h5>{{ s.title }}</h5>
          <span class="spacer"></span>
          <button type="button" class="btn" [disabled]="locked(s.scope)" (click)="write(s.scope)" [attr.data-testid]="'knowledge-new-' + s.scope" i18n="@@ai.knowledge.new">New…</button>
          <button type="button" class="btn" [disabled]="locked(s.scope)" (click)="importFile(s.scope)" [attr.data-testid]="'knowledge-import-' + s.scope" i18n="@@ai.knowledge.import">Import…</button>
        </div>
        <p class="muted small">{{ s.hint }}</p>
        <ul>
          @for (f of files(s.scope); track f.id) {
            <li [attr.data-testid]="'knowledge-file-' + f.name">
              <label class="toggle">
                <input type="checkbox" [checked]="f.enabled" [disabled]="locked(s.scope)" (change)="store.setEnabled(f, checked($event))" [attr.aria-label]="enableLabel(f)" />
                <span class="name">{{ f.name }}</span>
              </label>
              <span class="muted small">{{ size(f) }}</span>
              <span class="spacer"></span>
              <button type="button" class="btn ghost" [disabled]="locked(s.scope)" (click)="edit(f)" [attr.aria-label]="editLabel(f)" i18n="@@ai.knowledge.edit">Edit…</button>
              <button type="button" class="btn ghost" (click)="exportFile(f)" [attr.aria-label]="exportLabel(f)" i18n="@@ai.knowledge.export">Export…</button>
              <button type="button" class="btn ghost" [disabled]="locked(s.scope)" (click)="remove(f)" [attr.aria-label]="removeLabel(f)" i18n="@@ai.knowledge.remove">Remove…</button>
            </li>
          } @empty {
            <li class="muted empty" i18n="@@ai.knowledge.empty">No knowledge files yet.</li>
          }
        </ul>
      </section>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; margin: 12px 0; }
    h4 { margin: 0; font-size: var(--fs-md); font-weight: 600; }
    h5 { margin: 0; font-size: var(--fs-md); font-weight: 600; color: var(--text-2); }
    p { margin: 0; }
    .desc { color: var(--muted); line-height: 1.45; }
    .small { font-size: var(--fs-sm); }
    section { display: flex; flex-direction: column; gap: 4px; padding: 6px 0; border-top: 1px solid var(--border); }
    .head, li { display: flex; align-items: center; gap: 6px; }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
    li { padding: 2px 0; }
    .toggle { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
    .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .spacer { flex: 1; }
    .note { padding: 6px 8px; border-inline-start: 2px solid var(--warn-text); color: var(--text-2); }
    .banner { display: flex; align-items: center; gap: 10px; padding: 6px 10px; border-radius: 6px; border: 1px solid var(--danger-text); color: var(--danger-text); }
    .banner > span { flex: 1; }
  `,
})
export class KnowledgeSettings implements OnInit {
  protected readonly store = inject(KnowledgeStore);
  protected readonly persistence = inject(KnowledgePersistence);
  private readonly dialogs = inject(DialogService);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  protected readonly notice = signal('');

  protected readonly scopes: { scope: KnowledgeScope; title: string; hint: string }[] = [
    {
      scope: 'project',
      title: $localize`:@@ai.knowledge.scope.project:This project`,
      hint: $localize`:@@ai.knowledge.scope.projectHint:Saved with the project; the AI reads these first.`,
    },
    {
      scope: 'global',
      title: $localize`:@@ai.knowledge.scope.global:Every project`,
      hint: $localize`:@@ai.knowledge.scope.globalHint:Kept in IMT's settings folder on this computer.`,
    },
  ];

  constructor() {
    // A project opened or restored: list its files again.
    effect(() => {
      if (this.persistence.restored()) untracked(() => void this.store.load());
    });
  }

  ngOnInit(): void {
    void this.store.load();
  }

  protected files(scope: KnowledgeScope): KnowledgeInfo[] {
    return scope === 'project' ? this.store.project() : this.store.global();
  }

  protected locked(scope: KnowledgeScope): boolean {
    return this.store.busy() || (scope === 'global' && this.store.globalReadOnly());
  }

  protected size(f: KnowledgeInfo): string {
    return f.enabled ? formatBytes(f.bytes) : $localize`:@@ai.knowledge.disabledSize:${formatBytes(f.bytes)}:size: · not read by the AI`;
  }

  protected checked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected enableLabel(f: KnowledgeInfo): string {
    return $localize`:@@ai.knowledge.enableLabel:The AI reads ${f.name}:name:`;
  }
  protected editLabel(f: KnowledgeInfo): string {
    return $localize`:@@ai.knowledge.editLabel:Edit ${f.name}:name:`;
  }
  protected exportLabel(f: KnowledgeInfo): string {
    return $localize`:@@ai.knowledge.exportLabel:Export ${f.name}:name:`;
  }
  protected removeLabel(f: KnowledgeInfo): string {
    return $localize`:@@ai.knowledge.removeLabel:Remove ${f.name}:name:`;
  }

  private openEditor(file: KnowledgeInfo | null, scope: KnowledgeScope): void {
    openKnowledgeEditor(this.dialogs, file, scope, this.store.list()?.maxFileBytes ?? 64 * 1024);
  }

  write(scope: KnowledgeScope): void {
    this.openEditor(null, scope);
  }

  edit(f: KnowledgeInfo): void {
    this.openEditor(f, f.scope);
  }

  async importFile(scope: KnowledgeScope): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@ai.knowledge.importTitle:Import a knowledge file`, MARKDOWN);
    if (path) await this.store.importFile(scope, path);
  }

  async exportFile(f: KnowledgeInfo): Promise<void> {
    const name = f.name.replace(/[^\w.-]+/g, '-').slice(0, 60) || 'knowledge';
    const path = await this.platform.pickSavePath($localize`:@@ai.knowledge.exportTitle:Export knowledge file`, `${name}.md`, [{ name: 'Markdown', extensions: ['md'] }]);
    if (!path) return;
    try {
      const written = await this.overwrite.write(path, (o) => this.store.exportFile(f, path, o));
      if (written !== null) this.notice.set($localize`:@@ai.knowledge.exported:Exported to ${written}:path:`);
    } catch (e) {
      this.notice.set($localize`:@@ai.knowledge.notExported:Not exported: ${toAiError(e).message}:error:`);
    }
  }

  async remove(f: KnowledgeInfo): Promise<void> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@ai.knowledge.confirm.title:Remove knowledge file?`,
      width: 460,
      inputs: {
        message:
          f.scope === 'project'
            ? $localize`:@@ai.knowledge.confirm.project:Remove ${f.name}:name: from this project? The AI no longer reads it once you save the project.`
            : $localize`:@@ai.knowledge.confirm.global:Remove ${f.name}:name:? It is deleted from this computer and the AI no longer reads it in any project.`,
        confirmLabel: $localize`:@@ai.knowledge.confirm.remove:Remove`,
        danger: true,
      },
    });
    if ((await ref.closed) === true) await this.store.remove(f);
  }
}
