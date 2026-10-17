import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { FormatWizard } from '../format/format-wizard';
import { DialogService } from '../ui/dialog.service';
import { SourceInfo, UnrecognizedFile } from '../core/api.types';
import { formatCompact, formatCount, formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { FieldsPanel } from '../workspace/fields-panel';

/** Last folder + file name, e.g. "web01/orders.clef". */
function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
}

@Component({
  selector: 'imt-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, FieldsPanel],
  template: `
    <div class="head">
      <span class="section-title">Explorer</span>
      <span class="tools">
        <button class="icon-btn" aria-label="Open files" title="Open files" (click)="store.pickAndOpen(false)"><imt-icon name="file" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open folder" title="Open folder" (click)="store.pickAndOpen(true)"><imt-icon name="folder" [size]="14" /></button>
        <button class="icon-btn" aria-label="Re-import all" title="Re-import all" (click)="store.reload()"><imt-icon name="refresh" [size]="14" /></button>
      </span>
    </div>

    <div class="sources">
      @for (s of sources(); track s.id) {
        <div class="src" [title]="sourceTitle(s)">
          <imt-icon name="file" [size]="13" class="faint" />
          <span class="ellipsis name">{{ short(s.path) }}</span>
          <span class="badge">{{ s.importer }}</span>
          <span class="mono count">{{ compact(s.records) }}</span>
          <button class="icon-btn rm" [attr.aria-label]="'Remove ' + s.name" (click)="store.removeSource(s.id)" [attr.title]="'Remove ' + s.name"><imt-icon name="close" [size]="12" /></button>
        </div>
        @if (s.error) {
          <div class="src-error">{{ s.error }}</div>
        }
      } @empty {
        <div class="empty">
          <p>No files open.</p>
          <div class="open-row">
            <button class="btn primary" (click)="store.pickAndOpen(true)">Open folder…</button>
            <button class="btn" (click)="store.pickAndOpen(false)">Open files…</button>
          </div>
          <p class="faint small">Or drop log files anywhere in the window. Any extension works: the format is detected from the content.</p>
        </div>
      }
      @for (u of unrecognized(); track u.path) {
        <div class="src dim" [title]="u.reason">
          <imt-icon name="file" [size]="13" />
          <span class="ellipsis name">{{ short(u.path) }}</span>
          <span class="badge">—</span>
          <button class="btn ghost setup" [attr.aria-label]="'Set up format for ' + u.name" title="Teach IMT how to read this file" (click)="setUpFormat(u)">Set up format…</button>
        </div>
      }
    </div>

    <div class="divider"></div>

    <imt-fields-panel />
  `,
  styles: `
    :host { width: 272px; flex: none; display: flex; flex-direction: column; background: var(--bg-2); border-right: 1px solid var(--border); min-height: 0; }
    .head { height: 34px; flex: none; display: flex; align-items: center; justify-content: space-between; padding: 0 8px 0 12px; }
    .tools { display: flex; gap: 2px; }
    .small { font-size: 11px; }
    .sources { padding: 0 8px 4px; max-height: 40%; overflow: auto; flex: none; }
    .src { display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 2px 0 6px; border-radius: 5px; color: var(--text-2); }
    .src:hover { background: var(--bg-hover); }
    .src .name { flex: 1; min-width: 0; }
    .src .count { font-size: 11px; color: var(--faint); width: 38px; text-align: right; }
    .src .rm { width: 20px; height: 20px; visibility: hidden; }
    .src:hover .rm, .src .rm:focus-visible { visibility: visible; }
    .src.dim { color: var(--faint); font-style: italic; }
    .src.dim imt-icon { opacity: 0.6; }
    .setup { height: 20px; padding: 0 6px; font-size: 11px; font-style: normal; }
    .src-error { margin: 0 6px 4px 27px; font-size: 11px; color: var(--danger-text); }
    .empty { padding: 8px 6px; color: var(--muted); }
    .empty p { margin: 4px 0 10px; }
    .open-row { display: flex; gap: 6px; margin-bottom: 8px; }
    .divider { height: 1px; background: var(--border); margin: 6px 0; flex: none; }
  `,
})
export class Sidebar {
  protected readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  protected readonly compact = formatCompact;
  protected readonly short = shortPath;

  protected readonly sources = computed(() => this.store.summary()?.sources ?? []);
  protected readonly unrecognized = computed(() => this.store.summary()?.unrecognized ?? []);

  protected sourceTitle(s: SourceInfo): string {
    const tz = this.store.timezone();
    const dropped = s.dropped > 0 ? `\n${formatCount(s.dropped)} dropped by pipeline` : '';
    return `${s.path}\n${formatCount(s.records)} events · ${s.skipped} skipped${dropped}\n${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}`;
  }

  protected setUpFormat(file: UnrecognizedFile): void {
    this.dialogs.open(FormatWizard, { title: 'Set up format', inputs: { path: file.path, name: file.name }, width: 760 });
  }
}
