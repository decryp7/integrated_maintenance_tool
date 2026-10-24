import { RovingTabindex } from '../shared/roving-tabindex';
import { PathText } from '../shared/path-text';
import { ScrollFade } from '../shared/scroll-fade';
import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { DialogService } from '../ui/dialog.service';
import { EncodingInfo, SourceInfo, SourceProblem, TailFileState, UnrecognizedFile } from '../core/api.types';
import { formatCompact, formatCount, formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { FieldsPanel } from '../workspace/fields-panel';
import { groupSources } from '../live/source-groups';
import { explorerRows, splitMiddle } from './explorer-names';
import { NotableState, stateHint, stateLabel, TailStatusStore } from '../live/tail-status.store';
import { ContextMenuService } from '../ui/context-menu.service';
import { ClockDialogService } from '../skew/clock-section';
import { formatOffset } from '../skew/skew-format';
import { READ_ONLY_DETAIL, READ_ONLY_PROMISE } from '../shared/read-only';
import { SourceColors } from '../sources/source-colors.store';
import { SourceFileActions } from '../sources/source-file-actions';
import { SourceSwatch } from '../sources/source-swatch';
import { TimestampWarning } from './timestamp-warning';
import { ImportRange } from '../import/import-range';
import { AiLauncher } from '../ai/ai-launcher';
import { AiStatusStore } from '../ai/ai-status.store';
import { isImagePath } from '../ai/vision/image-files';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { isLowConfidence, encodingBadge, encodingTitle, showsEncodingBadge } from '../sources/encoding/encoding-format';
import { NoteImageStore } from '../notes/image/note-image.store';
import { anchors, NOTE_IMAGE_VIEW, NoteActions } from '../notes/note-actions';
import { NotesStore } from '../notes/notes.store';
import { ParsersStore } from '../settings/parsers/parsers.store';

type TailFileStateOf = Extract<TailFileState, { kind: NotableState }>;

/** An import problem of a source, in words (the first line it occurred on, when known). */
export function problemText(p: SourceProblem): string {
  const count = formatCount(p.count);
  const line = p.sampleLines[0];
  switch (p.kind) {
    case 'unmatchedLine':
      return line === undefined
        ? $localize`:@@shell.sidebar.problem.unmatched:${count}:count: lines look like events but match no format (kept with the event above)`
        : $localize`:@@shell.sidebar.problem.unmatchedAt:${count}:count: lines look like events but match no format (kept with the event above), first on line ${line}:line:`;
    case 'routeLimit':
      return $localize`:@@shell.sidebar.problem.routeLimit:${count}:count: events past the logical-source limit stayed in the file`;
    default:
      return $localize`:@@shell.sidebar.problem.other:${count}:count: import problems (${p.kind}:kind:)`;
  }
}

/** Last folder + file name, e.g. "web01/orders.clef". */
function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
}

@Component({
  selector: 'debrief-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RovingTabindex, PathText, ScrollFade, Icon, FieldsPanel, NgTemplateOutlet, SourceSwatch, TimestampWarning],
  template: `
    <div class="head">
      <span class="section-title" i18n="@@shell.sidebar.explorer">Explorer</span>
      <span class="tools" role="toolbar" aria-label="Explorer actions" i18n-aria-label="@@shell.sidebar.tools" debriefRoving="button" rovingAxis="horizontal">
        <button class="icon-btn" aria-label="Open files" title="Open files" i18n-aria-label="@@shell.sidebar.openFiles" i18n-title="@@shell.sidebar.openFilesTitle" (click)="store.pickAndOpen(false)"><debrief-icon name="file" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open folder" title="Open folder" i18n-aria-label="@@shell.sidebar.openFolder" i18n-title="@@shell.sidebar.openFolderTitle" (click)="store.pickAndOpen(true)"><debrief-icon name="folder" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open a time range" title="Open a time range…" i18n-aria-label="@@shell.sidebar.openRange" i18n-title="@@shell.sidebar.openRangeTitle" data-testid="open-range" (click)="importRange.pickAndOpen(true)"><debrief-icon name="metrics" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open for live tail" title="Open for live tail…" i18n-aria-label="@@shell.sidebar.openLive" i18n-title="@@shell.sidebar.openLiveTitle" (click)="openLive()"><debrief-icon name="timeline" [size]="14" /></button>
        <button class="icon-btn" aria-label="Re-import all" title="Re-import all" i18n-aria-label="@@shell.sidebar.reimportAll" i18n-title="@@shell.sidebar.reimportAllTitle" (click)="store.reload()"><debrief-icon name="refresh" [size]="14" /></button>
      </span>
    </div>

    <div class="sources" debriefScrollFade role="region" aria-label="Opened files" i18n-aria-label="@@shell.sidebar.openedFiles" [title]="readOnly" aria-describedby="debrief-sources-read-only">
      <span id="debrief-sources-read-only" class="sr">{{ readOnly }}</span>
      @if (layout().root; as root) {
        <div class="folder-root" data-testid="folder-root" [title]="root"><debrief-icon name="folder" [size]="12" /><span>{{ root }}</span></div>
      }
      @for (row of layout().rows; track row.item.kind === 'file' ? 'f' + row.item.source.id : 'g' + row.item.group.key) {
        @let item = row.item;
        @if (row.heading; as h) {
          <div class="folder-head" data-testid="folder-heading" [title]="h.path"><debrief-icon name="folder" [size]="12" /><span class="ellipsis">{{ h.label }}</span></div>
        }
        @if (item.kind === 'file') {
          <ng-container *ngTemplateOutlet="file; context: { $implicit: item.source, member: false, nameOnly: layout().byFolder }" />
        } @else {
          @let g = item.group;
          <div class="src group" [title]="g.split ? splitTitle(g.label, g.sources.length) : groupTitle(g.key, g.sources.length, g.newest.name)">
            <button
              type="button"
              class="expander"
              data-testid="source-group"
              [attr.aria-expanded]="isOpen(g.key)"
              (click)="toggle(g.key)"
            >
              <debrief-icon [name]="isOpen(g.key) ? 'chevronDown' : 'chevronRight'" [size]="12" />
              <debrief-icon name="files" [size]="13" class="faint" />
              <span class="ellipsis name">{{ g.label }}</span>
              @if (g.split) {
                <span class="badge" data-testid="split-badge" i18n="@@shell.sidebar.groupSources">{{ g.sources.length }} sources</span>
              } @else {
                <span class="badge" i18n="@@shell.sidebar.groupFiles">{{ g.sources.length }} files</span>
                <span class="sr" i18n="@@shell.sidebar.groupNewestSr">, newest {{ g.newest.name }}</span>
              }
            </button>
            <span class="count">{{ compact(g.records) }}</span>
          </div>
          @if (!isOpen(g.key)) {
            @if (!g.split) {
              <div class="newest faint ellipsis" i18n="@@shell.sidebar.groupNewest">newest: {{ g.newest.name }}</div>
            }
          } @else {
            <div role="group" [attr.aria-label]="filesOf(g.label)">
              @for (s of g.sources; track s.id) {
                <ng-container *ngTemplateOutlet="file; context: { $implicit: s, member: true, nameOnly: true }" />
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
        <div class="src dim unparsed" [title]="u.reason">
          <debrief-icon name="file" [size]="13" />
          <debrief-path-text class="name" [value]="short(u.path)" [full]="u.path" />
          @if (isImage(u.path)) {
            <button class="btn ghost setup" data-testid="mark-up-image" [attr.aria-label]="markUpLabel(u.name)" title="Open the image to mark areas and write notes" i18n-title="@@shell.sidebar.markUpTitle" (click)="markUp(u)" i18n="@@shell.sidebar.markUp">Mark up</button>
          }
          @if (canOpenImage(u.path)) {
            <button class="btn ghost setup" data-testid="open-image" [attr.aria-label]="openImageLabel(u.name)" title="Open the image for the AI assistant" i18n-title="@@shell.sidebar.openImageTitle" (click)="openImage(u)" i18n="@@shell.sidebar.openImage">Open image</button>
          } @else if (!isImage(u.path)) {
            <button class="btn ghost setup" [attr.aria-label]="setUpFormatLabel(u.name)" title="Teach Debrief how to read this file" i18n-title="@@shell.sidebar.setUpFormatTitle" (click)="setUpFormat(u)" i18n="@@shell.sidebar.setUpFormat">Set up format…</button>
          }
        </div>
      }
    </div>

    @if (sources().length) {
      <div class="divider"></div>
      <debrief-fields-panel />
    }

    <ng-template #file let-s let-member="member" let-nameOnly="nameOnly">
      <div class="src" [class.member]="member" [title]="sourceTitle(s)" (contextmenu)="sourceMenu($event, s)">
        @if (colors.forExplorer(s); as m) {
          <debrief-source-swatch [swatch]="m.swatch" [label]="m.label" [size]="12" />
        } @else {
          <debrief-icon name="file" [size]="13" class="faint" />
        }
        @let nm = splitName(member || nameOnly ? s.name : short(s.path));
        <span class="name" [title]="s.path"><span class="nm-head">{{ nm.head }}</span><span class="nm-tail">{{ nm.tail }}</span></span>
        @if (notable(s.id); as st) {
          <span class="state" [class]="'state ' + st.kind" [title]="hint(st)" [attr.aria-label]="label(st) + ': ' + hint(st)" role="img">{{ label(st) }}</span>
        }
        @if (s.offsetMs) {
          <span class="skew mono" data-testid="skew-badge" [title]="skewLabel(s.offsetMs)" [attr.aria-label]="skewLabel(s.offsetMs)" role="img">{{ offset(s.offsetMs) }}</span>
        }
        <debrief-timestamp-warning [source]="s" />
        @if (notes.bySource().get(s.path); as n) {
          <span class="notes-badge" data-testid="source-notes-badge" role="img" [title]="notesLabel(n)" [attr.aria-label]="notesLabel(n)"><debrief-icon name="note" [size]="11" />{{ n }}</span>
        }
        @if (encodingShown(s); as enc) {
          <span class="enc mono" data-testid="encoding-badge" [class.low]="isLowConfidence(enc)" role="img" [title]="encodingTitle(enc)" [attr.aria-label]="encodingTitle(enc)"><debrief-icon name="alert" [size]="12" /><span class="sr">{{ encodingBadge(enc) }}</span></span>
        }
        <span class="count">{{ compact(s.records) }}</span>
        <span class="actions">
        <button class="icon-btn rm" aria-haspopup="menu" data-testid="source-actions" [attr.aria-label]="actionsLabel(s.name)" [attr.title]="actionsLabel(s.name)" (click)="sourceMenu($event, s)"><debrief-icon name="drag" [size]="12" /></button>
        <button class="icon-btn rm" [attr.aria-label]="adjustClockLabel(s.name)" [attr.title]="adjustClockTitle(s.name)" (click)="adjustClock(s)"><debrief-icon name="timeline" [size]="12" /></button>
        <button class="icon-btn rm" [attr.aria-label]="removeLabel(s.name)" (click)="store.removeSource(s.id)" [attr.title]="removeLabel(s.name)"><debrief-icon name="close" [size]="12" /></button>
        </span>
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
    .small { font-size: var(--fs-xs); }
    .sources { padding: 0 8px 4px; max-height: 40%; overflow: auto; flex: none; }
    /* Rows that continue below the edge fade out instead of showing a half row. */
    .sources[data-more-below] { mask-image: linear-gradient(to bottom, var(--text) calc(100% - 20px), transparent); }
    .src { position: relative; display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 2px 0 6px; border-radius: 5px; color: var(--text-2); }
    .src:hover { background: var(--bg-hover); }
    /* Truncates with an ellipsis but never collapses to nothing next to the badge, count and row buttons. */
    .src .name { flex: 1; min-width: 0; display: flex; overflow: hidden; }
    .src .name .nm-head { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .src .name .nm-tail { flex: none; white-space: pre; }
    /* The folder every listed path shares, said once; a long one wraps instead of hiding its distinguishing end. */
    .folder-root { display: flex; align-items: flex-start; gap: 6px; padding: 4px 6px 0; font-size: var(--fs-xs); color: var(--text-2); overflow-wrap: anywhere; }
    .folder-root debrief-icon { flex: none; margin-top: 2px; }
    .folder-head { display: flex; align-items: center; gap: 6px; height: 22px; padding: 0 6px; margin-top: 4px; font-size: var(--fs-xs); color: var(--muted); font-weight: 600; }
    .group .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .src .count { flex: none; font-size: var(--fs-xs); color: var(--muted); min-width: 32px; text-align: end; font-variant-numeric: tabular-nums; }
    /* Row buttons overlay the count on hover or focus: they never take width from the name. */
    .src .actions { position: absolute; inset-block: 0; inset-inline-end: 2px; display: none; align-items: center; gap: 0; padding-inline-start: 8px; background: var(--bg-hover); border-radius: 5px; }
    .src:hover .actions, .src:focus-within .actions { display: flex; }
    .src .rm { width: 20px; height: 20px; }
    .src.dim { color: var(--faint); font-style: italic; }
    .src.dim debrief-icon { opacity: 0.6; }
    .src.unparsed .name { flex-basis: calc(100% - 22px); }
    .src.unparsed { flex-wrap: wrap; height: auto; min-height: 26px; padding-block: 2px; }
    .setup { height: 20px; padding: 0 6px; font-size: var(--fs-xs); font-style: normal; margin-inline-start: 21px; }
    .src-error { margin: 0 6px 4px 27px; font-size: var(--fs-xs); color: var(--danger-text); }
    .empty { padding: 8px 6px; color: var(--muted); }
    .empty p { margin: 4px 0 10px; }
    .divider { height: 1px; background: var(--border); margin: 6px 0; flex: none; }
    .src.member { padding-inline-start: 24px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .group .expander { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; height: 24px; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; text-align: start; }
    .group .expander:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; border-radius: 4px; }
    .newest { margin: -4px 6px 2px 46px; font-size: var(--fs-xs); }
    .state { flex: none; max-width: 9ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-2xs); padding: 0 5px; border-radius: 7px; border: 1px solid var(--border-2); color: var(--text-2); }
    .state.rotated, .state.truncated { border-color: var(--warn-text); color: var(--warn-text); }
    .state.deleted { border-style: dashed; }
    .state.error { border-color: var(--danger-text); color: var(--danger-text); }
    .notes-badge { flex: none; display: inline-flex; align-items: center; gap: 2px; font-size: var(--fs-2xs); color: var(--accent-text); }
    /* A guessed encoding is a small warning mark; the encoding itself is in the tooltip and read by screen readers. */
    .enc { flex: none; display: inline-flex; color: var(--faint); }
    .enc.low { color: var(--warn-text); }
    .skew { flex: none; font-size: var(--fs-2xs); padding: 0 5px; border-radius: 7px; border: 1px solid var(--accent); color: var(--text-2); }
  `,
})
export class Sidebar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly importRange = inject(ImportRange);
  private readonly dialogs = inject(DialogService);

  protected readonly compact = formatCompact;
  /** Tooltip of the source list: the read-only promise. */
  protected readonly readOnly = `${READ_ONLY_PROMISE} ${READ_ONLY_DETAIL}`;
  protected readonly short = shortPath;

  private readonly tail = inject(TailStatusStore);
  private readonly menu = inject(ContextMenuService);
  private readonly clock = inject(ClockDialogService);
  private readonly parsers = inject(ParsersStore);
  private readonly files = inject(SourceFileActions);
  protected readonly colors = inject(SourceColors);
  protected readonly offset = formatOffset;
  protected readonly sources = computed(() => this.store.summary()?.sources ?? []);
  /** Sources with rolling files of one log collapsed into expandable groups. */
  protected readonly items = computed(() => groupSources(this.sources()));
  /** The items in display order, with a folder heading before each folder when several are open (H2). */
  protected readonly layout = computed(() => explorerRows(this.items()));
  protected readonly splitName = splitMiddle;
  private readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly unrecognized = computed(() => this.store.summary()?.unrecognized ?? []);

  protected sourceTitle(s: SourceInfo): string {
    const tz = this.store.timezone();
    const count = formatCount(s.records);
    const skipped = s.skipped;
    const range = `${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}`;
    const lines = [s.path, $localize`:@@shell.sidebar.sourceTitle.format:Format: ${s.importer}:format:`, $localize`:@@shell.sidebar.sourceTitle.counts:${count}:count: events · ${skipped}:skipped: skipped`];
    if (s.dropped > 0) {
      const dropped = formatCount(s.dropped);
      lines.push($localize`:@@shell.sidebar.sourceTitle.dropped:${dropped}:dropped: dropped by pipeline`);
    }
    if (s.routedFrom) {
      const file = s.routedFrom;
      lines.push($localize`:@@shell.sidebar.sourceTitle.routedFrom:Split from ${file}:file: by a profile route`);
    }
    if (s.parser) {
      const parser = s.parser;
      lines.push($localize`:@@shell.sidebar.sourceTitle.parser:Read with the parser ${parser}:parser:`);
    }
    for (const p of s.problems ?? []) lines.push(problemText(p));
    lines.push(range);
    return lines.join('\n');
  }

  protected splitTitle(label: string, sources: number): string {
    return `${label}\n${$localize`:@@shell.sidebar.splitTitle:Split into ${sources}:sources: sources by profile routes`}`;
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
        { label: $localize`:@@shell.sidebar.menu.parseWith:Parse with…`, run: () => void this.parseWith(s) },
        ...(s.encoding ? [{ label: $localize`:@@shell.sidebar.menu.encoding:Encoding…`, run: () => void this.chooseEncoding(s) }] : []),
        { label: $localize`:@@shell.sidebar.menu.addNote:Add note…`, run: () => void this.noteActions.write(anchors.source(s.path)) },
        { label: $localize`:@@shell.sidebar.menu.adjustClock:Adjust clock…`, run: () => this.adjustClock(s) },
        { label: $localize`:Context menu item@@shell.sidebar.menu.remove:Remove`, run: () => void this.store.removeSource(s.id) },
      ],
      anchor,
    );
  }

  /** Explorer ▸ "Parse with…": opens the Parsers view to choose a parser for this source (ADR 0026, I3). */
  protected async parseWith(s: SourceInfo): Promise<void> {
    void this.parsers.chooseFor(s);
    this.commands.showView('parsers');
  }

  /** Explorer ▸ "Encoding…": read this source again with another text encoding. */
  protected async chooseEncoding(s: SourceInfo): Promise<void> {
    const { EncodingDialog } = await import('../sources/encoding/encoding-dialog');
    const name = s.name;
    this.dialogs.open(EncodingDialog, { title: $localize`:@@shell.sidebar.dialog.encoding:Encoding of ${name}:name:`, inputs: { source: s }, width: 520 });
  }

  /** The source's encoding when the row badges it (not for a confidently detected UTF-8 or a binary format). */
  protected encodingShown(s: SourceInfo): EncodingInfo | null {
    return showsEncodingBadge(s.encoding) ? s.encoding : null;
  }

  protected readonly isLowConfidence = isLowConfidence;
  protected readonly encodingTitle = encodingTitle;
  protected readonly encodingBadge = encodingBadge;

  protected async openLive(): Promise<void> {
    const { OpenLiveDialog } = await import('../live/open-live-dialog');
    this.dialogs.open(OpenLiveDialog, { title: $localize`:@@shell.sidebar.dialog.openLive:Open for live tail`, width: 480 });
  }

  private readonly ai = inject(AiStatusStore);
  private readonly aiLauncher = inject(AiLauncher);

  /** An image the local AI can read (ADR 0025): offered instead of a log format. */
  protected canOpenImage(path: string): boolean {
    return this.ai.available() && isImagePath(path);
  }

  protected openImageLabel(name: string): string {
    return $localize`:@@shell.sidebar.openImageLabel:Open image ${name}:name:`;
  }

  protected readonly notes = inject(NotesStore);
  private readonly noteActions = inject(NoteActions);
  private readonly noteImages = inject(NoteImageStore);
  private readonly commands = inject(COMMAND_CONTEXT);

  protected isImage(path: string): boolean {
    return isImagePath(path);
  }

  protected markUpLabel(name: string): string {
    return $localize`:@@shell.sidebar.markUpLabel:Mark up image ${name}:name:`;
  }

  protected notesLabel(n: number): string {
    return n === 1 ? $localize`:@@shell.sidebar.oneNote:1 note` : $localize`:@@shell.sidebar.notes:${n}:count: notes`;
  }

  /** Opens an image of the workspace in the image view to mark areas and write notes (ADR 0021). */
  protected markUp(file: UnrecognizedFile): void {
    this.commands.showView(NOTE_IMAGE_VIEW);
    void this.noteImages.showPath(file.path);
  }

  protected openImage(file: UnrecognizedFile): void {
    void this.aiLauncher.openImage(file.path);
  }

  protected setUpFormat(file: UnrecognizedFile): void {
    this.dialogs.openLazy(() => import('../format/format-wizard').then((m) => m.FormatWizard), { title: $localize`:@@shell.sidebar.dialog.setUpFormat:Set up format`, inputs: { path: file.path, name: file.name }, width: 760 });
  }
}
