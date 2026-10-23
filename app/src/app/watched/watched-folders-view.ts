import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Platform } from '../core/platform';
import { formatCount } from '../core/format';
import { WatchedFolderInfo } from '../core/api.types';
import { plural } from '../i18n/locale';
import { Icon } from '../shared/icon';
import { WatchedFoldersStore } from './watched-folders.store';

/** Folders the window watches for new files: add, pause and remove them (ADR 0048). */
@Component({
  selector: 'debrief-watched-folders-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar">
      <span class="title" i18n="@@watched.title">Watched folders</span>
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="add()" data-testid="watch-add" i18n="@@watched.add">Watch a folder…</button>
    </div>
    <p class="hint" i18n="@@watched.hint">Files that appear in these folders are imported into this project once they stop changing. Known issues from your knowledge files are announced when they show up.</p>
    @if (store.error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    @if (store.folders().length) {
      <ul class="list" aria-label="Watched folders" i18n-aria-label="@@watched.list">
        @for (f of store.folders(); track f.path) {
          <li class="item" [attr.data-testid]="'watched-' + name(f)">
            <input type="checkbox" [checked]="f.enabled" [attr.aria-label]="enableLabel(f)" (change)="store.setEnabled(f.path, checked($event))" />
            <span class="grow"><span class="mono ellipsis" [title]="f.path">{{ f.path }}</span>
              <span class="muted status">{{ status(f) }}</span></span>
            <button type="button" class="icon-btn" [attr.aria-label]="removeLabel(f)" title="Stop watching" i18n-title="@@watched.remove" (click)="store.remove(f.path)"><debrief-icon name="trash" [size]="13" /></button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint" i18n="@@watched.empty">No folders are watched.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; padding: 8px 12px; color: var(--text-2); font-size: var(--fs-sm); }
    .bar { display: flex; align-items: center; gap: 8px; }
    .title { font-weight: 600; color: var(--text); }
    .spacer { flex: 1; }
    p { margin: 0; }
    .err { color: var(--danger-text); }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .item { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .status { font-size: var(--fs-xs); }
  `,
})
export class WatchedFoldersView {
  protected readonly store = inject(WatchedFoldersStore);
  private readonly platform = inject(Platform);

  protected async add(): Promise<void> {
    const [path] = await this.platform.pickPaths(true);
    if (path) await this.store.add(path);
  }

  protected name(f: WatchedFolderInfo): string {
    return f.path.split(/[\\/]/).filter(Boolean).pop() ?? f.path;
  }

  protected checked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected status(f: WatchedFolderInfo): string {
    const n = f.importedFiles;
    const imported = plural(n, {
      one: $localize`:@@watched.status.importedOne:${formatCount(n)}:count: file imported`,
      other: $localize`:@@watched.status.importedMany:${formatCount(n)}:count: files imported`,
    });
    const w = f.pending;
    const waiting = w ? ` · ${$localize`:@@watched.status.pending:${formatCount(w)}:count: waiting`}` : '';
    const paused = f.enabled ? '' : ` · ${$localize`:@@watched.status.paused:paused`}`;
    return `${imported}${waiting}${paused}${f.message ? ` · ${f.message}` : ''}`;
  }

  protected enableLabel(f: WatchedFolderInfo): string {
    const name = this.name(f);
    return $localize`:@@watched.enableLabel:Watch the folder ${name}:name:`;
  }

  protected removeLabel(f: WatchedFolderInfo): string {
    const name = this.name(f);
    return $localize`:@@watched.removeLabel:Stop watching the folder ${name}:name:`;
  }
}
