import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, Injector, signal, viewChild } from '@angular/core';
import { CHANGE_KINDS, ChangeKind, DiffRow, SnapshotFile, ValueDto } from '../core/config-diff.types';
import { makeGeometry, scrollToTop, visibleWindow } from '../grid/scroll-map';
import { baseName } from '../tools/tool-errors';
import { ConfigDiffStore } from './config-diff.store';
import { ActiveDescendantNavigation } from '../shared/list-navigation';

/** Height of one diff row, px. */
export const ROW_H = 24;
const OVERSCAN = 8;
const SEARCH_DELAY_MS = 250;

const KIND_LABEL: Record<ChangeKind, string> = {
  added: $localize`:@@configDiff.view.kind.added:Added`,
  removed: $localize`:@@configDiff.view.kind.removed:Removed`,
  changed: $localize`:@@configDiff.view.kind.changed:Changed`,
  unchanged: $localize`:@@configDiff.view.kind.unchanged:Unchanged`,
};
const MARK: Record<ChangeKind, string> = { added: '+', removed: '−', changed: '~', unchanged: ' ' };

/** A row slot of the virtual window: the row, or null while its page loads. */
interface Slot {
  index: number;
  top: number;
  row: DiffRow | null;
}

/** Value as shown in a cell: sections have none, strings are quoted only when empty. */
export function valueText(v: ValueDto | null): string {
  if (!v || v.kind === 'section') return '';
  if (v.kind === 'string' && v.text === '') return '""';
  return v.text;
}

/**
 * Configuration diff: two files (registry exports, JSON/XML/INI/TOML/YAML, msinfo reports...) as a
 * side-by-side or unified tree diff, virtualised and paged from the engine; or two snapshot folders
 * of the same machine compared file by file.
 */
@Component({
  selector: 'imt-config-diff-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="wrap">
      <div class="modes" role="group" aria-label="Compare" i18n-aria-label="@@configDiff.view.modes">
        <button type="button" class="btn" [attr.aria-pressed]="store.mode() === 'files'" (click)="store.setMode('files')" i18n="@@configDiff.view.twoFiles">Two files</button>
        <button type="button" class="btn" [attr.aria-pressed]="store.mode() === 'folders'" (click)="store.setMode('folders')" i18n="@@configDiff.view.twoFolders">Two snapshot folders</button>
      </div>

      @if (store.mode() === 'files') {
        <div class="pick">
          <label class="side">
            <span class="lbl" i18n="@@configDiff.view.leftLabel">Left (original)</span>
            <input class="input mono" [value]="store.left()" (change)="store.setPath('left', value($event))" placeholder="Path to a configuration file" i18n-placeholder="@@configDiff.view.pathPlaceholder" />
          </label>
          <button type="button" class="btn" (click)="store.choose('left')" i18n="Pick a file or folder|Button@@configDiff.view.choose">Choose…</button>
          <button type="button" class="btn ghost" aria-label="Swap left and right" i18n-aria-label="@@configDiff.view.swapLabel" title="Swap left and right" i18n-title="@@configDiff.view.swapTitle" (click)="store.swap()">⇄</button>
          <label class="side">
            <span class="lbl" i18n="@@configDiff.view.rightLabel">Right (changed)</span>
            <input class="input mono" [value]="store.right()" (change)="store.setPath('right', value($event))" placeholder="Path to a configuration file" i18n-placeholder="@@configDiff.view.pathPlaceholder" />
          </label>
          <button type="button" class="btn" (click)="store.choose('right')" i18n="Pick a file or folder|Button@@configDiff.view.choose">Choose…</button>
          <button type="button" class="btn primary" [disabled]="!store.ready()" (click)="store.compare()" i18n="Run the comparison|Button@@configDiff.view.compare">Compare</button>
        </div>
      } @else {
        <div class="pick">
          <label class="side">
            <span class="lbl" i18n="@@configDiff.view.earlier">Earlier snapshot</span>
            <input class="input mono" [value]="store.leftDir()" (change)="store.setDir('left', value($event))" placeholder="Folder, e.g. a collector run" i18n-placeholder="@@configDiff.view.folderPlaceholder" />
          </label>
          <button type="button" class="btn" (click)="store.chooseFolder('left')" i18n="Pick a file or folder|Button@@configDiff.view.choose">Choose…</button>
          <label class="side">
            <span class="lbl" i18n="@@configDiff.view.later">Later snapshot</span>
            <input class="input mono" [value]="store.rightDir()" (change)="store.setDir('right', value($event))" placeholder="Folder, e.g. a collector run" i18n-placeholder="@@configDiff.view.folderPlaceholder" />
          </label>
          <button type="button" class="btn" (click)="store.chooseFolder('right')" i18n="Pick a file or folder|Button@@configDiff.view.choose">Choose…</button>
          <button type="button" class="btn primary" [disabled]="!store.leftDir() || !store.rightDir() || store.snapshotBusy()" (click)="store.compareFolders()" i18n="@@configDiff.view.compareFolders">Compare folders</button>
        </div>
      }

      <details class="opts">
        <summary i18n="@@configDiff.view.opts">Ignore patterns and options</summary>
        <label class="ign">
          <span class="lbl" i18n="@@configDiff.view.ignoreLabel">Ignore paths matching (one per line; globs like <code>*InstallDate*</code>, or <code>re:</code> regexes)</span>
          <textarea class="input mono" rows="4" [value]="store.ignoreText()" (change)="store.setIgnoreText(value($event))"></textarea>
        </label>
        <div class="row-opts">
          <button type="button" class="btn ghost" (click)="store.resetIgnore()" i18n="@@configDiff.view.resetIgnore">Reset to defaults</button>
          <label class="check" i18n="@@configDiff.view.ignoreCase"><input type="checkbox" [checked]="store.caseInsensitive()" (change)="store.setCaseInsensitive(checked($event))" /> Ignore case in paths</label>
          @if (store.formats().length) {
            <span class="faint formats" i18n="@@configDiff.view.formats">Formats: {{ formatNames() }}</span>
          }
        </div>
      </details>

      @if (store.error(); as e) {
        <div class="alert" role="alert">
          <span>{{ e }}</span>
          <button type="button" class="btn" (click)="store.compare()" [disabled]="!store.ready()" i18n="@@configDiff.view.retry">Retry</button>
          <button type="button" class="btn ghost" (click)="store.dismiss()" i18n="@@configDiff.view.dismiss">Dismiss</button>
        </div>
      }
      @if (store.notice(); as n) {
        <div class="notice" role="status">{{ n }}</div>
      }

      @if (store.mode() === 'folders') {
        @if (store.snapshotError(); as e) {
          <div class="alert" role="alert"><span>{{ e }}</span> <button type="button" class="btn" (click)="store.compareFolders()" i18n="@@configDiff.view.retry">Retry</button></div>
        }
        @if (store.snapshot(); as snap) {
          <table class="files" aria-label="Configuration files in both snapshots" i18n-aria-label="@@configDiff.view.filesTable">
            <thead><tr><th scope="col" i18n="@@configDiff.view.col.file">File</th><th scope="col" i18n="@@configDiff.view.col.status">Status</th><th scope="col" i18n="@@configDiff.view.col.format">Format</th><th scope="col" class="num" i18n="@@configDiff.view.col.changes">Changes</th><th scope="col"><span class="sr" i18n="@@configDiff.view.col.action">Action</span></th></tr></thead>
            <tbody>
              @for (f of snap.files; track f.relPath) {
                <tr [attr.data-status]="f.status">
                  <td class="mono">{{ f.relPath }}</td>
                  <td><span class="st" [class]="'st ' + f.status">{{ f.status }}</span>@if (f.error) { <span class="faint err"> {{ f.error }}</span> }</td>
                  <td>{{ f.format ?? '' }}</td>
                  <td class="num mono">{{ f.counts ? f.counts.added + f.counts.removed + f.counts.changed : '' }}</td>
                  <td>@if (f.leftPath && f.rightPath && f.status !== 'unreadable') { <button type="button" class="btn" [attr.aria-label]="showDiffLabel(f.relPath)" (click)="open(f)" i18n="@@configDiff.view.showDiff">Show diff</button> }</td>
                </tr>
              } @empty {
                <tr><td colspan="5" class="faint" i18n="@@configDiff.view.noFiles">No configuration files found in these folders.</td></tr>
              }
            </tbody>
          </table>
          @if (snap.truncated) { <p class="faint" i18n="@@configDiff.view.truncated">Only the first files of each folder were compared.</p> }
        } @else if (store.snapshotBusy()) {
          <p class="faint" role="status" i18n="@@configDiff.view.comparingFolders">Comparing folders…</p>
        } @else {
          <p class="faint" i18n="@@configDiff.view.chooseFolders">Choose two folders, for example two diagnostics-collector runs of the same machine, to see which configuration files changed.</p>
        }
      } @else {
        @if (store.meta(); as m) {
          <div class="bar">
            <div class="kinds" role="group" aria-label="Show changes" i18n-aria-label="@@configDiff.view.showChanges">
              @for (k of kinds; track k) {
                <label class="check kind" [attr.data-kind]="k">
                  <input type="checkbox" [checked]="store.kinds().includes(k)" (change)="store.toggleKind(k)" />
                  <span class="mark" aria-hidden="true">{{ mark[k] }}</span> {{ label[k] }} <span class="mono count">{{ m.counts[k] }}</span>
                </label>
              }
              @if (m.counts.ignored) { <span class="faint" i18n="@@configDiff.view.ignored">{{ m.counts.ignored }} ignored</span> }
            </div>
            <input class="input search" type="search" aria-label="Search paths and values" i18n-aria-label="@@configDiff.view.searchLabel" placeholder="Search paths and values" i18n-placeholder="@@configDiff.view.searchPlaceholder" [value]="store.search()" (input)="onSearch(value($event))" />
            <label class="check" i18n="@@configDiff.view.expandChanged"><input type="checkbox" [checked]="store.expand() === 'changedOnly'" (change)="store.setExpand(checked($event) ? 'changedOnly' : 'all')" /> Expand changed only</label>
            <div class="seg" role="group" aria-label="Layout" i18n-aria-label="@@configDiff.view.layout">
              <button type="button" class="btn" [attr.aria-pressed]="store.layout() === 'split'" (click)="store.setLayout('split')" i18n="@@configDiff.view.split">Side by side</button>
              <button type="button" class="btn" [attr.aria-pressed]="store.layout() === 'unified'" (click)="store.setLayout('unified')" i18n="@@configDiff.view.unified">Unified</button>
            </div>
            <button type="button" class="btn" (click)="store.exportAs('csv')" i18n="@@configDiff.view.exportCsv">Export CSV</button>
            <button type="button" class="btn" (click)="store.exportAs('html')" i18n="@@configDiff.view.exportHtml">Export HTML</button>
          </div>
          <div class="sides faint mono">
            <span title="{{ m.left.path }}" i18n="@@configDiff.view.leftSummary">− {{ base(m.left.path) }} ({{ m.left.format }}, {{ m.left.nodes }} nodes)</span>
            <span title="{{ m.right.path }}" i18n="@@configDiff.view.rightSummary">+ {{ base(m.right.path) }} ({{ m.right.format }}, {{ m.right.nodes }} nodes)</span>
            @if (m.caseInsensitive) { <span i18n="@@configDiff.view.pathsIgnoreCase">paths ignore case</span> }
            @if (m.truncated) { <span class="warn" i18n="@@configDiff.view.nodeLimit">a file is larger than the node limit; the rest is not compared</span> }
          </div>
        }

        <div class="tree-box">
        <div
          #scroller
          class="tree"
          [class.unified]="store.layout() === 'unified'"
          role="treegrid"
          aria-label="Configuration diff" i18n-aria-label="@@configDiff.view.tree"
          [attr.aria-rowcount]="store.total()"
          [attr.aria-activedescendant]="activeId()"
          [attr.aria-busy]="store.loading()"
          tabindex="0"
          (scroll)="onScroll()"
          (keydown)="onKey($event)"
        >
          <div class="sizer" role="rowgroup" [style.height.px]="geometry().physicalH">
            @for (s of slots(); track s.index) {
              @if (s.row; as r) {
                <div
                  class="r"
                  role="row"
                  [id]="'cd-row-' + s.index"
                  [attr.data-kind]="r.kind"
                  [class.ctx]="r.context"
                  [class.sel]="store.selected() === s.index"
                  [attr.aria-level]="r.depth + 1"
                  [attr.aria-expanded]="r.expandable ? r.expanded : null"
                  [attr.aria-selected]="store.selected() === s.index"
                  [attr.aria-rowindex]="s.index + 1"
                  [style.top.px]="s.top"
                  (click)="store.select(s.index)"
                >
                  @if (store.layout() === 'split') {
                    <div class="c left" role="gridcell" [style.padding-left.px]="8 + r.depth * 14">
                      <ng-container *ngTemplateOutlet="toggleTpl; context: { $implicit: r }" />
                      @if (r.old) { <span class="name">{{ r.name }}</span><span class="val" [attr.data-type]="r.old.kind">{{ text(r.old) }}</span> }
                    </div>
                    <div class="c right" role="gridcell" [style.padding-left.px]="8 + r.depth * 14">
                      @if (r.new) { <span class="name">{{ r.name }}</span><span class="val" [attr.data-type]="r.new.kind">{{ text(r.new) }}</span> }
                    </div>
                  } @else {
                    <div class="c one" role="gridcell" [style.padding-left.px]="8 + r.depth * 14">
                      <ng-container *ngTemplateOutlet="toggleTpl; context: { $implicit: r }" />
                      <span class="mark" aria-hidden="true">{{ mark[r.kind] }}</span>
                      <span class="name">{{ r.name }}</span>
                      @if (r.kind === 'changed') {
                        <span class="val old">{{ text(r.old) }}</span><span class="arrow" aria-hidden="true">→</span><span class="sr" i18n="@@configDiff.view.changedTo">changed to</span><span class="val new">{{ text(r.new) }}</span>
                      } @else {
                        <span class="val">{{ text(r.new ?? r.old) }}</span>
                      }
                    </div>
                  }
                </div>
              } @else {
                <div class="r loading" role="row" [style.top.px]="s.top" [attr.aria-rowindex]="s.index + 1"><div class="c" role="gridcell" i18n="@@configDiff.view.loading">Loading…</div></div>
              }
            }
          </div>
        </div>
        @if (!store.meta() && !store.error()) {
          <p class="empty faint">@if (store.loading()) {<ng-container i18n="@@configDiff.view.comparing">Comparing…</ng-container>} @else {<ng-container i18n="@@configDiff.view.chooseFiles">Choose two configuration files to compare.</ng-container>}</p>
        } @else if (store.meta() && store.total() === 0) {
          <p class="empty faint" i18n="@@configDiff.view.noDifferences">No differences to show with these filters.</p>
        }
        </div>

        @if (store.selectedRow(); as r) {
          <div class="details" role="group" aria-label="Selected row" i18n-aria-label="@@configDiff.view.selectedRow">
            <div class="d-path mono" [title]="r.path">{{ r.path || rootLabel }}</div>
            <button type="button" class="btn" (click)="store.copy('path', r.path)" i18n="@@configDiff.view.copyPath">Copy path</button>
            @if (r.old && r.old.kind !== 'section') { <button type="button" class="btn" (click)="store.copy('old value', r.old.text)" i18n="@@configDiff.view.copyOld">Copy old value</button> }
            @if (r.new && r.new.kind !== 'section') { <button type="button" class="btn" (click)="store.copy('new value', r.new.text)" i18n="@@configDiff.view.copyNew">Copy new value</button> }
          </div>
        }
      }
    </div>

    <ng-template #toggleTpl let-r>
      @if (r.expandable) {
        <button type="button" class="tog" tabindex="-1" [attr.aria-label]="toggleLabel(r.expanded, r.path)" (click)="toggle($event, r.path)">{{ r.expanded ? '▾' : '▸' }}</button>
      } @else {
        <span class="tog" aria-hidden="true"></span>
      }
    </ng-template>
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .wrap { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; }
    .modes, .seg { display: flex; gap: 4px; }
    .btn[aria-pressed='true'] { background: var(--bg-sel); border-color: var(--accent); }
    .pick { display: flex; gap: 6px; align-items: flex-end; flex-wrap: wrap; }
    .side { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 180px; }
    .lbl { font-size: 11.5px; color: var(--text-2); }
    .opts summary { cursor: pointer; color: var(--text-2); font-size: 12px; }
    .ign { display: flex; flex-direction: column; gap: 4px; margin-top: 6px; }
    .ign textarea { width: 100%; box-sizing: border-box; }
    .row-opts { display: flex; gap: 12px; align-items: center; margin-top: 6px; flex-wrap: wrap; }
    .check { display: inline-flex; gap: 4px; align-items: center; font-size: 12px; }
    .alert { display: flex; gap: 10px; align-items: center; padding: 6px 10px; border: 1px solid var(--lvl-error); border-radius: 6px; background: var(--bg-2); color: var(--text); }
    .notice { font-size: 12px; color: var(--text-2); }
    .bar { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
    .kinds { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
    .count { color: var(--text-2); }
    .search { min-width: 200px; flex: 1; }
    .sides { display: flex; gap: 16px; font-size: 11.5px; flex-wrap: wrap; }
    .warn { color: var(--warn-text); }
    .tree-box { position: relative; flex: 1; min-height: 160px; display: flex; }
    .tree { position: relative; flex: 1; min-height: 0; overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-0); font-family: var(--font-mono); font-size: 12px; }
    .tree:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .sizer { position: relative; min-width: 100%; }
    .r { position: absolute; left: 0; right: 0; height: ${ROW_H}px; display: grid; grid-template-columns: 1fr 1fr; cursor: default; color: var(--text); }
    .unified .r { grid-template-columns: 1fr; }
    .r.sel { outline: 1px solid var(--accent); outline-offset: -1px; background: var(--bg-sel); }
    .r.ctx { color: var(--text-2); }
    .c { display: flex; align-items: center; gap: 6px; min-width: 0; overflow: hidden; white-space: nowrap; border-right: 1px solid var(--grid-line); }
    .name { color: inherit; }
    .val { color: var(--text); overflow: hidden; text-overflow: ellipsis; }
    .val[data-type='section'] { display: none; }
    .val[data-type='integer'], .val[data-type='float'], .val[data-type='bool'], .val[data-type='null'] { color: var(--accent-ink, var(--text)); }
    .name::after { content: ':'; color: var(--text-2); }
    .r[data-kind='added'] .right, .unified .r[data-kind='added'] .one { background: color-mix(in srgb, var(--ok) 16%, transparent); }
    .r[data-kind='removed'] .left, .unified .r[data-kind='removed'] .one { background: color-mix(in srgb, var(--lvl-error) 16%, transparent); }
    .r[data-kind='changed'] .c { background: color-mix(in srgb, var(--lvl-warn) 16%, transparent); }
    .r[data-kind='added'] .left, .r[data-kind='removed'] .right { background: repeating-linear-gradient(135deg, transparent 0 6px, var(--bg-2) 6px 8px); }
    .old { text-decoration: line-through; color: var(--text-2); }
    .arrow { color: var(--text-2); }
    .mark { width: 10px; text-align: center; font-weight: 600; }
    .tog { width: 16px; height: 16px; padding: 0; border: none; background: none; color: var(--text-2); cursor: pointer; flex: none; font-size: 11px; line-height: 16px; }
    .loading .c { color: var(--text-2); padding-left: 8px; }
    .empty { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; margin: 0; font-family: var(--font-sans); pointer-events: none; }
    .details { display: flex; gap: 8px; align-items: center; padding: 4px 0; }
    .d-path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
    .files { border-collapse: collapse; font-size: 12px; width: 100%; }
    .files th, .files td { padding: 3px 8px; text-align: left; border-bottom: 1px solid var(--border); }
    .files .num { text-align: right; }
    .st.changed { color: var(--warn-text); }
    .st.unreadable, .st.removed { color: var(--danger-text); }
    .err { font-size: 11px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
    .formats { font-size: 11.5px; }
  `,
})
export class ConfigDiffView {
  protected showDiffLabel(path: string): string {
    return $localize`:@@configDiff.view.showDiffOf:Show diff of ${path}:path:`;
  }
  protected readonly store = inject(ConfigDiffStore);
  protected readonly kinds = CHANGE_KINDS;
  protected readonly label = KIND_LABEL;
  protected readonly mark = MARK;
  protected readonly text = valueText;
  protected readonly base = baseName;
  protected readonly rootLabel = $localize`:@@configDiff.view.root:(root)`;
  private readonly scroller = viewChild<ElementRef<HTMLElement>>('scroller');
  private readonly scrollTop = signal(0);
  private readonly viewportH = signal(400);
  private searchTimer: ReturnType<typeof setTimeout> | undefined;
  /** Moves the selected row (CDK ActiveDescendantKeyManager: focus stays on the tree, which names the row). */
  private readonly nav = new ActiveDescendantNavigation(this.store.total, inject(Injector), {
    homeAndEnd: true,
    pageDelta: () => Math.max(1, Math.floor(this.viewportH() / ROW_H) - 1),
  });

  protected readonly formatNames = computed(() => this.store.formats().map((f) => f.name).join(', '));
  protected readonly geometry = computed(() => makeGeometry(this.store.total(), ROW_H, this.viewportH()));
  /** The rows in view (plus overscan), positioned inside the scrolled content. */
  protected readonly slots = computed<Slot[]>(() => {
    const g = this.geometry();
    const scroll = this.scrollTop();
    const top = scrollToTop(g, scroll);
    const { first, last } = visibleWindow(g, top, OVERSCAN);
    this.store.pages();
    const out: Slot[] = [];
    for (let i = first; i < last; i++) out.push({ index: i, top: scroll + i * ROW_H - top, row: this.store.rowAt(i) });
    return out;
  });
  protected readonly activeId = computed(() => {
    const i = this.store.selected();
    return i === null || !this.store.rowAt(i) ? null : `cd-row-${i}`;
  });

  constructor() {
    void this.store.init();
    // Load the pages of the rows in view whenever the view or the scroll position changes.
    effect(() => {
      const slots = this.slots();
      this.store.generation();
      const first = slots[0];
      const last = slots[slots.length - 1];
      if (first && last && slots.some((s) => !s.row)) void this.store.ensure(first.index, last.index + 1);
    });
    effect((onCleanup) => {
      const host = this.scroller()?.nativeElement;
      if (!host) return;
      const measure = () => this.viewportH.set(host.clientHeight || 400);
      measure();
      if (typeof ResizeObserver === 'undefined') return;
      const ro = new ResizeObserver(measure);
      ro.observe(host);
      onCleanup(() => ro.disconnect());
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.searchTimer));
  }

  protected toggleLabel(expanded: boolean, path: string): string {
    return expanded
      ? $localize`:@@configDiff.view.collapse:Collapse ${path}:path:`
      : $localize`:@@configDiff.view.expand:Expand ${path}:path:`;
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  protected checked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected onScroll(): void {
    this.scrollTop.set(this.scroller()?.nativeElement.scrollTop ?? 0);
  }

  protected onSearch(text: string): void {
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => void this.store.setSearch(text), SEARCH_DELAY_MS);
  }

  protected toggle(e: Event, path: string): void {
    e.stopPropagation();
    void this.store.toggle(path);
  }

  protected open(f: SnapshotFile): void {
    void this.store.openFile(f);
  }

  /** Arrow keys move the selection; Right/Left expand/collapse; Enter toggles; Home/End jump. */
  protected onKey(e: KeyboardEvent): void {
    const total = this.store.total();
    if (!total) return;
    const cur = this.store.selected();
    const row = cur === null ? null : this.store.rowAt(cur);
    let next: number;
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
      case 'Home':
      case 'End':
      case 'PageDown':
      case 'PageUp':
        // Nothing selected yet: the arrows start at the first row.
        if (cur === null && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) next = 0;
        else {
          this.nav.sync(cur ?? 0);
          if (!this.nav.handle(e)) return;
          next = this.nav.active();
        }
        break;
      case 'ArrowRight':
      case 'ArrowLeft':
      case 'Enter':
        if (row?.expandable && (e.key === 'Enter' || row.expanded === (e.key === 'ArrowLeft'))) void this.store.toggle(row.path);
        e.preventDefault();
        return;
      default:
        return;
    }
    e.preventDefault();
    this.store.select(next);
    this.reveal(next);
  }

  /** Scrolls so row `i` is in view. */
  private reveal(i: number): void {
    const host = this.scroller()?.nativeElement;
    const g = this.geometry();
    if (!host || g.scaled) return;
    const y = i * ROW_H;
    if (y < host.scrollTop) host.scrollTop = y;
    else if (y + ROW_H > host.scrollTop + this.viewportH()) host.scrollTop = y + ROW_H - this.viewportH();
    this.onScroll();
  }
}
