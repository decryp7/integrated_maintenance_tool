import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, Injector, linkedSignal, signal, viewChildren } from '@angular/core';
import { Severity } from '../core/api.types';
import { formatCount } from '../core/format';
import { RovingFocusNavigation } from '../shared/list-navigation';
import { Icon } from '../shared/icon';
import { DialogService } from '../ui/dialog.service';
import { countText, evaluationText, evaluationTone, kindHint, kindLabel, parserCanFix, ProblemGroup, ProblemItem } from './problems-model';
import { ProblemsStore } from './problems.store';

/** One visible row of the tree: a file, or one of its problems. */
type TreeRow = { kind: 'group'; id: string; group: ProblemGroup; expanded: boolean; pos: number; size: number } | { kind: 'problem'; id: string; item: ProblemItem; pos: number; size: number };

/** Severity in words (the icon's accessible text). */
export function severityLabel(s: Severity): string {
  switch (s) {
    case 'critical':
      return $localize`:@@problems.severity.critical:Error`;
    case 'warning':
      return $localize`:@@problems.severity.warning:Warning`;
    case 'info':
      return $localize`:@@problems.severity.info:Information`;
  }
}

/**
 * The Problems pane, like an IDE's: what went wrong while reading the files (events without a readable timestamp,
 * lines that are not events, files no parser reads...), grouped by file and kind with counts. The details of the
 * selected problem jump to its lines, preview the engine's quick fix on the stored events, apply it (the file's
 * parser changes and its files are read again) or ask the AI for a parser to review.
 */
@Component({
  selector: 'imt-problems-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar">
      <span class="summary" data-testid="problems-summary">{{ summary() }}</span>
      <span class="spacer"></span>
      <button type="button" class="icon-btn" data-testid="problems-refresh" aria-label="Refresh problems" i18n-aria-label="@@problems.refresh" title="Refresh" i18n-title="@@problems.refreshTitle" [disabled]="store.loading()" (click)="store.load()">
        <imt-icon name="refresh" [size]="13" />
      </button>
    </div>
    @if (store.index(); as idx) {
      @if (!idx.ready) {
        <div class="index" data-testid="problems-index" i18n="@@problems.index">Search index: {{ fmt(idx.indexed) }} of {{ fmt(idx.total) }} events indexed; search covers the indexed events.</div>
      }
    }
    @if (store.loadError(); as error) {
      <div class="error" role="alert">
        <span i18n="@@problems.loadError">The problems could not be listed: {{ error }}</span>
        <button type="button" class="btn" (click)="store.load()" i18n="@@problems.retry">Retry</button>
      </div>
    }
    @if (store.actionError(); as error) {
      <div class="error" role="alert" data-testid="problems-action-error">
        <span>{{ error }}</span>
        <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@problems.dismiss" (click)="store.dismiss()"><imt-icon name="close" [size]="12" /></button>
      </div>
    }
    @if (store.notice(); as notice) {
      <div class="notice" role="status">{{ notice }}</div>
    }
    @if (store.loaded() && rows().length === 0 && !store.loadError()) {
      <div class="empty faint" data-testid="problems-empty" i18n="@@problems.empty">No problems: every opened file was read completely.</div>
    } @else {
      <div class="body">
        <div class="tree" role="tree" aria-label="Problems" i18n-aria-label="@@problems.tree" (keydown)="onKey($event)">
          @for (r of rows(); track r.id) {
            @if (r.kind === 'group') {
              <div
                #row
                class="item group"
                role="treeitem"
                aria-level="1"
                [attr.aria-posinset]="r.pos"
                [attr.aria-setsize]="r.size"
                [attr.aria-expanded]="r.expanded"
                aria-selected="false"
                [attr.data-id]="r.id"
                [attr.tabindex]="r.id === focusId() ? 0 : -1"
                [title]="r.group.path"
                (click)="activate(r)"
              >
                <imt-icon class="twisty" [name]="r.expanded ? 'chevronDown' : 'chevronRight'" [size]="12" />
                <imt-icon [class]="'sev ' + r.group.severity" name="alert" [size]="13" />
                <span class="name ellipsis">{{ r.group.name }}</span>
                <span class="count">{{ r.group.items.length }}</span>
              </div>
            } @else {
              <div
                #row
                class="item problem"
                role="treeitem"
                aria-level="2"
                data-testid="problem-row"
                [attr.aria-posinset]="r.pos"
                [attr.aria-setsize]="r.size"
                [attr.aria-selected]="r.item.key === store.selectedKey()"
                [attr.data-id]="r.id"
                [attr.tabindex]="r.id === focusId() ? 0 : -1"
                [class.sel]="r.item.key === store.selectedKey()"
                (click)="activate(r)"
              >
                <imt-icon [class]="'sev ' + r.item.row.severity" name="alert" [size]="13" />
                <span class="sr-only">{{ severity(r.item.row.severity) }}:</span>
                <span class="name ellipsis">{{ kind(r.item.row.kind) }}</span>
                <span class="count">{{ count(r.item) }}</span>
                @if (r.item.row.fix) {
                  <span class="fixable" i18n="@@problems.fixable">quick fix</span>
                }
              </div>
            }
          }
        </div>
        <section class="details" aria-label="Problem details" i18n-aria-label="@@problems.details">
          @if (store.selected(); as sel) {
            <h3 class="title" data-testid="problem-title">
              <imt-icon [class]="'sev ' + sel.row.severity" name="alert" [size]="14" />
              <span>{{ kind(sel.row.kind) }}</span>
              <span class="count">{{ count(sel) }}</span>
            </h3>
            <div class="path mono ellipsis" [title]="sel.row.path">{{ sel.row.path }}</div>
            <p class="hint">{{ hint(sel.row.kind) }}</p>
            @if (sel.row.detail) {
              <p class="detail">{{ sel.row.detail }}</p>
            }
            @if (sel.row.sampleLines.length) {
              <div class="lines">
                <span class="label" i18n="@@problems.lines">First lines:</span>
                @for (line of sel.row.sampleLines; track line) {
                  <button type="button" class="link mono" data-testid="problem-line" [attr.aria-label]="goToLabel(line)" (click)="store.jumpTo(sel.key, line)">{{ line }}</button>
                }
              </div>
            }
            @if (sel.row.fix; as fix) {
              <div class="fix" data-testid="quick-fix">
                <div class="fix-title"><imt-icon name="check" [size]="12" /> <span i18n="@@problems.fix.title">Quick fix: {{ fix.title }}</span></div>
                <pre class="fragment mono" aria-label="Parser change" i18n-aria-label="@@problems.fix.fragment">{{ fix.parserOverride }}</pre>
                @if (preview(sel.key); as p) {
                  @if (p.state === 'running') {
                    <div class="eval faint" role="status" i18n="@@problems.fix.checking">Checking the fix on the stored events…</div>
                  } @else if (p.state === 'done') {
                    <div class="eval" role="status" data-testid="fix-evaluation" [attr.data-tone]="tone(p.result)">{{ evaluation(p.result) }}</div>
                    @for (note of p.result.notes ?? []; track $index) {
                      <div class="note faint">{{ note }}</div>
                    }
                  } @else {
                    <div class="eval bad" role="alert" i18n="@@problems.fix.failed">The fix could not be checked: {{ p.error }}</div>
                  }
                }
                <div class="actions">
                  <button type="button" class="btn" data-testid="fix-preview" [disabled]="preview(sel.key)?.state === 'running'" (click)="store.preview(sel.key)" i18n="@@problems.fix.preview">Preview effect</button>
                  <button type="button" class="btn primary" data-testid="fix-apply" [disabled]="!!store.applying() || sel.row.source === null" (click)="store.apply(sel.key)">{{ store.applying() === sel.key ? applyingLabel : applyLabel }}</button>
                </div>
              </div>
            }
            @if (canAskAi(sel)) {
              <div class="actions">
                <button type="button" class="btn" data-testid="ask-ai-fix" (click)="store.askAi(sel.key)"><imt-icon name="ai" [size]="12" /> <span i18n="@@problems.askAi">Ask the AI to fix…</span></button>
                @if (sel.row.kind === 'unknownFormat') {
                  <button type="button" class="btn" (click)="setUpFormat(sel)" i18n="@@problems.setUpFormat">Set up format…</button>
                }
              </div>
              <p class="faint small" i18n="@@problems.askAiHint">The AI proposes a parser; you review it and the engine checks it before anything changes.</p>
            }
          } @else {
            <p class="faint" i18n="@@problems.selectHint">Select a problem to see its lines and fixes.</p>
          }
        </section>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); container-type: inline-size; }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 4px 12px; border-bottom: 1px solid var(--border); font-size: var(--fs-sm); color: var(--text-2); }
    .spacer { flex: 1; }
    .index, .notice { flex: none; padding: 4px 12px; font-size: var(--fs-sm); color: var(--text-2); border-bottom: 1px solid var(--border); }
    .error { flex: none; display: flex; align-items: center; gap: 8px; padding: 4px 12px; color: var(--danger-text); font-size: var(--fs-sm); border-bottom: 1px solid var(--border); }
    .error span { flex: 1; }
    .empty { padding: 12px 14px; }
    .body { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(220px, 45%) 1fr; }
    @container (max-width: 560px) { .body { grid-template-columns: 1fr; grid-template-rows: 1fr auto; } .details { border-left: 0; border-top: 1px solid var(--border); } }
    .tree { min-height: 0; overflow: auto; padding: 2px 0; }
    .item { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 10px; cursor: default; color: var(--text-2); white-space: nowrap; font-size: var(--fs-sm); }
    .item.problem { padding-left: 30px; }
    .item.group { color: var(--text); font-weight: 600; }
    .item:hover { background: var(--bg-hover); }
    .item.sel { background: var(--bg-sel); color: var(--text); }
    .item:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .twisty { flex: none; }
    .name { flex: 1; min-width: 0; }
    .count { flex: none; color: var(--muted); font-weight: 400; font-size: var(--fs-xs); }
    .fixable { flex: none; font-size: var(--fs-2xs); color: var(--accent-text); border: 1px solid currentColor; border-radius: 8px; padding: 0 5px; }
    .sev { flex: none; }
    .sev.critical { color: var(--lvl-error); }
    .sev.warning { color: var(--lvl-warn); }
    .sev.info { color: var(--accent); }
    .details { min-height: 0; overflow: auto; padding: 8px 14px; border-left: 1px solid var(--border); font-size: var(--fs-sm); }
    .title { display: flex; align-items: center; gap: 6px; margin: 0 0 4px; font-size: var(--fs-md); }
    .path { color: var(--muted); font-size: var(--fs-xs); }
    .hint, .detail { margin: 6px 0; color: var(--text-2); }
    .lines { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px; margin: 6px 0; }
    .lines .label { color: var(--muted); }
    .link { min-width: 24px; min-height: 24px; border: 0; background: transparent; color: var(--accent-text); cursor: pointer; padding: 0 4px; font-size: var(--fs-sm); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .fix { margin: 10px 0; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .fix-title { display: flex; align-items: center; gap: 6px; font-weight: 600; }
    .fragment { margin: 6px 0; padding: 6px 8px; background: var(--bg-0); border-radius: 4px; white-space: pre-wrap; word-break: break-all; font-size: var(--fs-xs); max-height: 120px; overflow: auto; }
    .eval { margin: 4px 0; }
    .eval[data-tone=good] { color: var(--ok); }
    .eval[data-tone=partial] { color: var(--warn-text); }
    .eval[data-tone=none], .eval.bad { color: var(--danger-text); }
    .note { font-size: var(--fs-xs); }
    .actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
    .small { font-size: var(--fs-xs); margin: 4px 0; }
  `,
})
export class ProblemsPanel {
  protected readonly store = inject(ProblemsStore);
  private readonly dialogs = inject(DialogService);
  private readonly rowEls = viewChildren<ElementRef<HTMLElement>>('row');

  protected readonly fmt = formatCount;
  protected readonly kind = kindLabel;
  protected readonly hint = kindHint;
  protected readonly severity = severityLabel;
  protected readonly evaluation = evaluationText;
  protected readonly tone = evaluationTone;
  protected readonly applyLabel = $localize`:@@problems.fix.apply:Apply and re-read`;
  protected readonly applyingLabel = $localize`:@@problems.fix.applying:Applying…`;

  /** Files whose problems are folded away (all start expanded). */
  protected readonly collapsed = signal<ReadonlySet<string>>(new Set());

  constructor() {
    // Workspace changes are listed sooner while this pane is visible.
    this.store.gate.use();
  }

  protected readonly rows = computed<TreeRow[]>(() => {
    const groups = this.store.groups();
    const out: TreeRow[] = [];
    groups.forEach((group, gi) => {
      const expanded = !this.collapsed().has(group.key);
      out.push({ kind: 'group', id: group.key, group, expanded, pos: gi + 1, size: groups.length });
      if (expanded) group.items.forEach((item, i) => out.push({ kind: 'problem', id: item.key, item, pos: i + 1, size: group.items.length }));
    });
    return out;
  });

  /** The roving tab stop; falls back to the first row when its row is gone (collapse, reload). */
  protected readonly focusId = linkedSignal<TreeRow[], string | null>({
    source: () => this.rows(),
    computation: (rows, previous) => {
      const focus = previous?.value;
      return focus && rows.some((r) => r.id === focus) ? focus : (rows[0]?.id ?? null);
    },
  });

  /** Up, Down, Home and End through the CDK FocusKeyManager; Left and Right fold and unfold files. */
  private readonly nav = new RovingFocusNavigation(computed(() => this.rows().length), inject(Injector), (i) => this.focusRow(i), { homeAndEnd: true });

  protected readonly summary = computed(() => {
    const c = this.store.counts();
    if (!this.store.loaded()) return $localize`:@@problems.loading:Reading the problems…`;
    const warnings = formatCount(c.warning);
    const infos = formatCount(c.info);
    const errors = formatCount(c.critical);
    return c.critical > 0
      ? $localize`:@@problems.summary.errors:${errors}:errors: errors, ${warnings}:warnings: warnings, ${infos}:infos: messages`
      : $localize`:@@problems.summary:${warnings}:warnings: warnings, ${infos}:infos: messages`;
  });

  protected count(item: ProblemItem): string {
    return countText(item.row);
  }

  protected preview(key: string) {
    return this.store.previews()[key] ?? null;
  }

  protected canAskAi(item: ProblemItem): boolean {
    return parserCanFix(item.row.kind);
  }

  protected goToLabel(line: number): string {
    return $localize`:@@problems.goToLine:Go to line ${line}:line:`;
  }

  protected activate(r: TreeRow): void {
    this.focusId.set(r.id);
    if (r.kind === 'group') this.toggle(r.group.key);
    else this.store.select(r.item.key);
  }

  private toggle(key: string, open?: boolean): void {
    const next = new Set(this.collapsed());
    const expand = open ?? next.has(key);
    if (expand) next.delete(key);
    else next.add(key);
    this.collapsed.set(next);
  }

  protected onKey(e: KeyboardEvent): void {
    const list = this.rows();
    const index = list.findIndex((r) => r.id === this.focusId());
    const current = list[index];
    if (!current) return;
    this.nav.sync(index);
    if (this.nav.handle(e)) {
      e.preventDefault();
      return;
    }
    switch (e.key) {
      case 'ArrowRight':
        if (current.kind === 'group' && !current.expanded) this.toggle(current.group.key, true);
        else if (current.kind === 'group') this.focusRow(index + 1);
        break;
      case 'ArrowLeft':
        if (current.kind === 'group' && current.expanded) this.toggle(current.group.key, false);
        else if (current.kind === 'problem') {
          let parent = index - 1;
          while (parent >= 0 && list[parent]?.kind !== 'group') parent--;
          if (parent >= 0) this.focusRow(parent);
        }
        break;
      case 'Enter':
      case ' ':
        this.activate(current);
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  /** Moves the tab stop and focus to row `i` of the visible tree. */
  private focusRow(i: number): void {
    const target = this.rows()[i];
    if (!target) return;
    this.focusId.set(target.id);
    queueMicrotask(() => this.rowEls().find((el) => el.nativeElement.dataset['id'] === target.id)?.nativeElement.focus());
  }

  /** "Set up format…" for a file no parser reads: the format wizard. */
  protected async setUpFormat(item: ProblemItem): Promise<void> {
    const { FormatWizard } = await import('../format/format-wizard');
    this.dialogs.open(FormatWizard, {
      title: $localize`:@@problems.setUpFormat.title:Set up format`,
      inputs: { path: item.row.path, name: item.row.sourceName },
      width: 760,
    });
  }
}
