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
import { ContextMenuService } from '../ui/context-menu.service';
import { ClockDialogService } from '../skew/clock-section';
import { formatOffset } from '../skew/skew-format';
import { READ_ONLY_DETAIL, READ_ONLY_PROMISE } from '../shared/read-only';
import { SourceColors } from '../sources/source-colors.store';
import { SourceFileActions } from '../sources/source-file-actions';
import { SourceSwatch } from '../sources/source-swatch';
import { TimestampWarning } from './timestamp-warning';

type TailFileStateOf = Extract<TailFileState, { kind: NotableState }>;

/** Last folder + file name, e.g. "web01/orders.clef". */
function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
}

@Component({
  selector: 'imt-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, FieldsPanel, NgTemplateOutlet, SourceSwatch, TimestampWarning],
  template: `
    <div class="head">
      <span class="section-title" i18n="@@shell.sidebar.explorer">Explorer</span>
      <span class="tools">
        <button class="icon-btn" aria-label="Open files" title="Open files" i18n-aria-label="@@shell.sidebar.openFiles" i18n-title="@@shell.sidebar.openFilesTitle" (click)="store.pickAndOpen(false)"><imt-icon name="file" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open folder" title="Open folder" i18n-aria-label="@@shell.sidebar.openFolder" i18n-title="@@shell.sidebar.openFolderTitle" (click)="store.pickAndOpen(true)"><imt-icon name="folder" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open for live tail" title="Open for live tail…" i18n-aria-label="@@shell.sidebar.openLive" i18n-title="@@shell.sidebar.openLiveTitle" (click)="openLive()"><imt-icon name="timeline" [size]="14" /></button>
        <button class="icon-btn" aria-label="Re-import all" title="Re-import all" i18n-aria-label="@@shell.sidebar.reimportAll" i18n-title="@@shell.sidebar.reimportAllTitle" (click)="store.reload()"><imt-icon name="refresh" [size]="14" /></button>
      </span>
    </div>

    <div class="sources" role="region" aria-label="Opened files" i18n-aria-label="@@shell.sidebar.openedFiles" [title]="readOnly" aria-describedby="imt-sources-read-only">
      <span id="imt-sources-read-only" class="sr">{{ readOnly }}</span>
      @for (item of items(); track item.kind === 'file' ? 'f' + item.source.id : 'g' + item.group.key) {
        @if (item.kind === 'file') {
          <ng-container *ngTemplateOutlet="file; context: { $implicit: item.source, member: false }" />
        } @else {
          @let g = item.group;
          <div class="src group" [title]="groupTitle(g.key, g.sources.length, g.newest.name)">
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
              <span class="badge" i18n="@@shell.sidebar.groupFiles">{{ g.sources.length }} files</span>
              <span class="sr" i18n="@@shell.sidebar.groupNewestSr">, newest {{ g.newest.name }}</span>
            </button>
            <span class="mono count">{{ compact(g.records) }}</span>
          </div>
          @if (!isOpen(g.key)) {
            <div class="newest faint ellipsis" i18n="@@shell.sidebar.groupNewest">newest: {{ g.newest.name }}</div>
          } @else {
            <div role="group" [attr.aria-label]="filesOf(g.label)">
              @for (s of g.sources; track s.id) {
                <ng-container *ngTemplateOutlet="file; context: { $implicit: s, member: true }" />
              }
            </div>
          }
        }
      } @empty {
        <div class="empty">
          <!-- Opening is offered once, on the welcome page (and by the buttons above). -->
          <p i18n="@@shell.sidebar.empty.none">No files open.</p>
        </div>
      }
      @for (u of unrecognized(); track u.path) {
        <div class="src dim" [title]="u.reason">
          <imt-icon name="file" [size]="13" />
          <span class="ellipsis name">{{ short(u.path) }}</span>
          <span class="badge">—</span>
          <button class="btn ghost setup" [attr.aria-label]="setUpFormatLabel(u.name)" title="Teach IMT how to read this file" i18n-title="@@shell.sidebar.setUpFormatTitle" (click)="setUpFormat(u)" i18n="@@shell.sidebar.setUpFormat">Set up format…</button>
        </div>
      }
    </div>

    @if (sources().length) {
      <div class="divider"></div>
      <imt-fields-panel />
    }

    <ng-template #file let-s let-member="member">
      <div class="src" [class.member]="member" [title]="sourceTitle(s)" (contextmenu)="sourceMenu($event, s)">
        @if (colors.forExplorer(s); as m) {
          <imt-source-swatch [swatch]="m.swatch" [label]="m.label" [size]="12" />
        } @else {
          <imt-icon name="file" [size]="13" class="faint" />
        }
        <span class="ellipsis name">{{ member ? s.name : short(s.path) }}</span>
        @if (notable(s.id); as st) {
          <span class="state" [class]="'state ' + st.kind" [title]="hint(st)" [attr.aria-label]="label(st) + ': ' + hint(st)" role="img">{{ label(st) }}</span>
        }
        @if (s.offsetMs) {
          <span class="skew mono" data-testid="skew-badge" [title]="skewLabel(s.offsetMs)" [attr.aria-label]="skewLabel(s.offsetMs)" role="img">{{ offset(s.offsetMs) }}</span>
        }
        <imt-timestamp-warning [source]="s" />
        <span class="badge">{{ s.importer }}</span>
        <span class="mono count">{{ compact(s.records) }}</span>
        <button class="icon-btn rm" aria-haspopup="menu" data-testid="source-actions" [attr.aria-label]="actionsLabel(s.name)" [attr.title]="actionsLabel(s.name)" (click)="sourceMenu($event, s)"><imt-icon name="drag" [size]="12" /></button>
        <button class="icon-btn rm" [attr.aria-label]="adjustClockLabel(s.name)" [attr.title]="adjustClockTitle(s.name)" (click)="adjustClock(s)"><imt-icon name="timeline" [size]="12" /></button>
        <button class="icon-btn rm" [attr.aria-label]="removeLabel(s.name)" (click)="store.removeSource(s.id)" [attr.title]="removeLabel(s.name)"><imt-icon name="close" [size]="12" /></button>
      </div>
      @if (s.error) {
        <div class="src-error">{{ s.error }}</div>
      }
    </ng-template>
  `,
  styles: `
    :host { width: 272px; flex: none; display: flex; flex-direction: column; background: var(--bg-2); border-inline-end: 1px solid var(--border); min-height: 0; }
    .head { height: 34px; flex: none; display: flex; align-items: center; justify-content: space-between; padding: 0 8px 0 12px; }
    .tools { display: flex; gap: 2px; }
    .small { font-size: 11px; }
    .sources { padding: 0 8px 4px; max-height: 40%; overflow: auto; flex: none; }
    .src { display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 2px 0 6px; border-radius: 5px; color: var(--text-2); }
    .src:hover { background: var(--bg-hover); }
    /* Truncates with an ellipsis but never collapses to nothing next to the badge, count and row buttons. */
    .src .name { flex: 1; min-width: 6ch; }
    .src .count { font-size: 11px; color: var(--faint); width: 38px; text-align: end; }
    .src .rm { width: 20px; height: 20px; visibility: hidden; }
    .src:hover .rm, .src .rm:focus-visible { visibility: visible; }
    .src.dim { color: var(--faint); font-style: italic; }
    .src.dim imt-icon { opacity: 0.6; }
    .setup { height: 20px; padding: 0 6px; font-size: 11px; font-style: normal; }
    .src-error { margin: 0 6px 4px 27px; font-size: 11px; color: var(--danger-text); }
    .empty { padding: 8px 6px; color: var(--muted); }
    .empty p { margin: 4px 0 10px; }
    .divider { height: 1px; background: var(--border); margin: 6px 0; flex: none; }
    .src.member { padding-inline-start: 24px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .group .expander { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; height: 24px; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; text-align: start; }
    .group .expander:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; border-radius: 4px; }
    .newest { margin: -4px 6px 2px 46px; font-size: 11px; }
    .state { flex: none; font-size: 10px; padding: 0 5px; border-radius: 7px; border: 1px solid var(--border-2); color: var(--text-2); }
    .state.rotated, .state.truncated { border-color: var(--warn-text); color: var(--warn-text); }
    .state.deleted { border-style: dashed; }
    .state.error { border-color: var(--danger-text); color: var(--danger-text); }
    .skew { flex: none; font-size: 10px; padding: 0 5px; border-radius: 7px; border: 1px solid var(--accent); color: var(--text-2); }
  `,
})
export class Sidebar {
  protected readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  protected readonly compact = formatCompact;
  /** Tooltip of the source list: the read-only promise. */
  protected readonly readOnly = `${READ_ONLY_PROMISE} ${READ_ONLY_DETAIL}`;
  protected readonly short = shortPath;

  private readonly tail = inject(TailStatusStore);
  private readonly menu = inject(ContextMenuService);
  private readonly clock = inject(ClockDialogService);
  private readonly files = inject(SourceFileActions);
  protected readonly colors = inject(SourceColors);
  protected readonly offset = formatOffset;
  protected readonly sources = computed(() => this.store.summary()?.sources ?? []);
  /** Sources with rolling files of one log collapsed into expandable groups. */
  protected readonly items = computed(() => groupSources(this.sources()));
  private readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly unrecognized = computed(() => this.store.summary()?.unrecognized ?? []);

  protected sourceTitle(s: SourceInfo): string {
    const tz = this.store.timezone();
    const count = formatCount(s.records);
    const skipped = s.skipped;
    const range = `${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}`;
    const lines = [s.path, $localize`:@@shell.sidebar.sourceTitle.counts:${count}:count: events · ${skipped}:skipped: skipped`];
    if (s.dropped > 0) {
      const dropped = formatCount(s.dropped);
      lines.push($localize`:@@shell.sidebar.sourceTitle.dropped:${dropped}:dropped: dropped by pipeline`);
    }
    lines.push(range);
    return lines.join('\n');
  }

  protected groupTitle(key: string, files: number, newest: string): string {
    return `${key}\n${$localize`:@@shell.sidebar.groupTitle:${files}:files: files · newest: ${newest}:newest:`}`;
  }

  protected filesOf(label: string): string {
    return $localize`:@@shell.sidebar.filesOf:Files of ${label}:label:`;
  }

  protected setUpFormatLabel(name: string): string {
    return $localize`:@@shell.sidebar.setUpFormatLabel:Set up format for ${name}:name:`;
  }

  protected skewLabel(offsetMs: number): string {
    const offset = this.offset(offsetMs);
    return $localize`:@@shell.sidebar.skew:Clock shifted by ${offset}:offset:`;
  }

  protected adjustClockLabel(name: string): string {
    return $localize`:@@shell.sidebar.adjustClockLabel:Adjust clock of ${name}:name:`;
  }

  protected adjustClockTitle(name: string): string {
    return $localize`:@@shell.sidebar.adjustClockTitle:Adjust clock of ${name}:name:…`;
  }

  protected actionsLabel(name: string): string {
    return $localize`:@@shell.sidebar.actionsLabel:Actions for ${name}:name:`;
  }

  protected removeLabel(name: string): string {
    return $localize`:@@shell.sidebar.removeLabel:Remove ${name}:name:`;
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

  /** Sources › Adjust clock… (clock-skew correction). */
  protected adjustClock(s: SourceInfo): void {
    this.clock.open(s.id, s.name);
  }

  protected sourceMenu(event: MouseEvent, s: SourceInfo): void {
    event.preventDefault();
    const anchor = event.currentTarget as HTMLElement | null;
    // A keyboard "click" has no pointer position: open below the button instead.
    const r = event.clientX || event.clientY ? null : anchor?.getBoundingClientRect();
    this.menu.open(
      r ? r.left : event.clientX,
      r ? r.bottom : event.clientY,
      [
        ...this.files.items(s),
        { label: $localize`:@@shell.sidebar.menu.adjustClock:Adjust clock…`, run: () => this.adjustClock(s) },
        { label: $localize`:Context menu item@@shell.sidebar.menu.remove:Remove`, run: () => void this.store.removeSource(s.id) },
      ],
      anchor,
    );
  }

  protected async openLive(): Promise<void> {
    const { OpenLiveDialog } = await import('../live/open-live-dialog');
    this.dialogs.open(OpenLiveDialog, { title: $localize`:@@shell.sidebar.dialog.openLive:Open for live tail`, width: 480 });
  }

  protected setUpFormat(file: UnrecognizedFile): void {
    this.dialogs.open(FormatWizard, { title: $localize`:@@shell.sidebar.dialog.setUpFormat:Set up format`, inputs: { path: file.path, name: file.name }, width: 760 });
  }
}
