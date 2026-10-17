import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormatWizard } from '../format/format-wizard';
import { DialogService } from '../ui/dialog.service';
import { SourceInfo, TailFileState, UnrecognizedFile } from '../core/api.types';
import { formatCompact, formatCount, formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { FieldsPanel } from '../workspace/fields-panel';
import { groupSources } from '../live/source-groups';
import { NotableState, stateHint, stateLabel, TailStatusStore } from '../live/tail-status.store';

type TailFileStateOf = Extract<TailFileState, { kind: NotableState }>;

/** Last folder + file name, e.g. "web01/orders.clef". */
function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
}

@Component({
  selector: 'imt-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, FieldsPanel, NgTemplateOutlet],
  template: `
    <div class="head">
      <span class="section-title">Explorer</span>
      <span class="tools">
        <button class="icon-btn" aria-label="Open files" title="Open files" (click)="store.pickAndOpen(false)"><imt-icon name="file" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open folder" title="Open folder" (click)="store.pickAndOpen(true)"><imt-icon name="folder" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open for live tail" title="Open for live tail…" (click)="openLive()"><imt-icon name="timeline" [size]="14" /></button>
        <button class="icon-btn" aria-label="Re-import all" title="Re-import all" (click)="store.reload()"><imt-icon name="refresh" [size]="14" /></button>
      </span>
    </div>

    <div class="sources">
      @for (item of items(); track item.kind === 'file' ? 'f' + item.source.id : 'g' + item.group.key) {
        @if (item.kind === 'file') {
          <ng-container *ngTemplateOutlet="file; context: { $implicit: item.source, member: false }" />
        } @else {
          @let g = item.group;
          <div class="src group" [title]="g.key + '\n' + g.sources.length + ' files · newest: ' + g.newest.name">
            <button
              type="button"
              class="expander"
              data-testid="source-group"
              [attr.aria-expanded]="isOpen(g.key)"
              (click)="toggle(g.key)"
            >
              <imt-icon [name]="isOpen(g.key) ? 'chevronDown' : 'chevronRight'" [size]="12" />
              <imt-icon name="files" [size]="13" class="faint" />
              <span class="ellipsis name">{{ g.label }}</span>
              <span class="badge">{{ g.sources.length }} files</span>
              <span class="sr">, newest {{ g.newest.name }}</span>
            </button>
            <span class="mono count">{{ compact(g.records) }}</span>
          </div>
          @if (!isOpen(g.key)) {
            <div class="newest faint ellipsis">newest: {{ g.newest.name }}</div>
          } @else {
            <div role="group" [attr.aria-label]="'Files of ' + g.label">
              @for (s of g.sources; track s.id) {
                <ng-container *ngTemplateOutlet="file; context: { $implicit: s, member: true }" />
              }
            </div>
          }
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

    <ng-template #file let-s let-member="member">
      <div class="src" [class.member]="member" [title]="sourceTitle(s)">
        <imt-icon name="file" [size]="13" class="faint" />
        <span class="ellipsis name">{{ member ? s.name : short(s.path) }}</span>
        @if (notable(s.id); as st) {
          <span class="state" [class]="'state ' + st.kind" [title]="hint(st)" [attr.aria-label]="label(st) + ': ' + hint(st)" role="img">{{ label(st) }}</span>
        }
        <span class="badge">{{ s.importer }}</span>
        <span class="mono count">{{ compact(s.records) }}</span>
        <button class="icon-btn rm" [attr.aria-label]="'Remove ' + s.name" (click)="store.removeSource(s.id)" [attr.title]="'Remove ' + s.name"><imt-icon name="close" [size]="12" /></button>
      </div>
      @if (s.error) {
        <div class="src-error">{{ s.error }}</div>
      }
    </ng-template>
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
    .src.member { padding-left: 24px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .group .expander { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; height: 24px; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; text-align: left; }
    .group .expander:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; border-radius: 4px; }
    .newest { margin: -4px 6px 2px 46px; font-size: 11px; }
    .state { flex: none; font-size: 10px; padding: 0 5px; border-radius: 7px; border: 1px solid var(--border-2); color: var(--text-2); }
    .state.rotated, .state.truncated { border-color: var(--warn-text); color: var(--warn-text); }
    .state.deleted { border-style: dashed; }
    .state.error { border-color: var(--danger-text); color: var(--danger-text); }
  `,
})
export class Sidebar {
  protected readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  protected readonly compact = formatCompact;
  protected readonly short = shortPath;

  private readonly tail = inject(TailStatusStore);
  protected readonly sources = computed(() => this.store.summary()?.sources ?? []);
  /** Sources with rolling files of one log collapsed into expandable groups. */
  protected readonly items = computed(() => groupSources(this.sources()));
  private readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly unrecognized = computed(() => this.store.summary()?.unrecognized ?? []);

  protected sourceTitle(s: SourceInfo): string {
    const tz = this.store.timezone();
    const dropped = s.dropped > 0 ? `\n${formatCount(s.dropped)} dropped by pipeline` : '';
    return `${s.path}\n${formatCount(s.records)} events · ${s.skipped} skipped${dropped}\n${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}`;
  }

  protected isOpen(key: string): boolean {
    return this.expanded().has(key);
  }

  protected toggle(key: string): void {
    this.expanded.update((set) => {
      const next = new Set(set);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }

  /** Rotated, truncated, deleted or failing file (live tail), for a badge. */
  protected notable(id: number): TailFileStateOf | null {
    return (this.tail.notable().get(id) as TailFileStateOf | undefined) ?? null;
  }

  protected label(state: TailFileStateOf): string {
    return stateLabel(state);
  }

  protected hint(state: TailFileStateOf): string {
    return stateHint(state);
  }

  protected async openLive(): Promise<void> {
    const { OpenLiveDialog } = await import('../live/open-live-dialog');
    this.dialogs.open(OpenLiveDialog, { title: 'Open for live tail', width: 480 });
  }

  protected setUpFormat(file: UnrecognizedFile): void {
    this.dialogs.open(FormatWizard, { title: 'Set up format', inputs: { path: file.path, name: file.name }, width: 760 });
  }
}
