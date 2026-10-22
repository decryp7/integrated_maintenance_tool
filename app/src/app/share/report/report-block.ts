import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { DomSanitizer, SafeUrl } from '@angular/platform-browser';
import { ReportBlock } from '../../core/api.types';
import { formatFull, valueToString } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { MarkdownView } from './markdown-view';
import { ReportStore } from './report.store';

const LABELS: Record<string, string> = {
  text: $localize`:@@share.reportBlock.kind.text:Text`,
  events: $localize`:@@share.reportBlock.kind.events:Events`,
  chart: $localize`:@@share.reportBlock.kind.chart:Chart`,
  timeline: $localize`:@@share.reportBlock.kind.timeline:Timeline`,
  table: $localize`:@@share.reportBlock.kind.table:Query result`,
  finding: $localize`:@@share.reportBlock.kind.finding:Finding`,
  annotations: $localize`:@@share.reportBlock.kind.annotations:Annotations`,
};

const SEVERITY_LABELS: Record<string, string> = {
  critical: $localize`:@@share.reportBlock.severity.critical:critical`,
  warning: $localize`:@@share.reportBlock.severity.warning:warning`,
  info: $localize`:@@share.reportBlock.severity.info:info`,
};

/** An SVG snapshot as an image URL: shown in an <img>, which never runs anything inside the SVG. */
export function svgImageUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/**
 * One block of the report builder: its content (text editable, the rest shown with an editable caption)
 * and move/remove actions. Alt+ArrowUp/Down on the focused block moves it.
 */
@Component({
  selector: 'debrief-report-block',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkdownView],
  host: {
    class: 'block',
    tabindex: '0',
    role: 'group',
    '[attr.aria-label]': 'label()',
    '[attr.data-block-id]': 'block().id',
    '(keydown)': 'key($event)',
  },
  template: `
    <div class="head">
      <span class="kind">{{ kindLabel() }}</span>
      <span class="spacer"></span>
      <button type="button" class="icon-btn" [attr.aria-label]="moveUpLabel()" title="Move up (Alt+↑)" i18n-title="@@share.reportBlock.moveUpTitle" [disabled]="first()" (click)="move(-1)"><span aria-hidden="true">↑</span></button>
      <button type="button" class="icon-btn" [attr.aria-label]="moveDownLabel()" title="Move down (Alt+↓)" i18n-title="@@share.reportBlock.moveDownTitle" [disabled]="last()" (click)="move(1)"><span aria-hidden="true">↓</span></button>
      <button type="button" class="icon-btn" [attr.aria-label]="removeLabel()" title="Remove" i18n-title="@@share.reportBlock.removeTitle" (click)="reports.removeBlock(block().id)"><debrief-icon name="trash" [size]="13" /></button>
    </div>
    @switch (block().type) {
      @case ('text') {
        <textarea class="input text" rows="4" [attr.aria-label]="textLabel()" placeholder="Write Markdown: # heading, **bold**, - list…" i18n-placeholder="@@share.reportBlock.textPlaceholder" [value]="$any(block()).markdown" (change)="patch({ markdown: val($event) })"></textarea>
        <debrief-markdown-view class="md" [text]="$any(block()).markdown" />
      }
      @case ('events') {
        <ul class="ids" aria-label="Events in this block" i18n-aria-label="@@share.reportBlock.eventsAria">
          @for (id of $any(block()).ids; track id) {
            <li><button type="button" class="btn link" [attr.aria-label]="showEventLabel(id)" (click)="ws.jumpTo(id)">#{{ id }}</button></li>
          }
        </ul>
      }
      @case ('finding') {
        <p class="finding"><strong [class]="'sev ' + $any(block()).severity">{{ severityLabel($any(block()).severity) }}</strong> {{ $any(block()).title }}</p>
        <p class="faint detail">{{ $any(block()).detail }}</p>
      }
      @case ('table') {
        @if ($any(block()).query) { <p class="mono faint q">{{ $any(block()).query }}</p> }
        <div class="table-wrap">
          <table>
            <thead><tr>@for (c of $any(block()).columns; track $index) { <th scope="col">{{ c }}</th> }</tr></thead>
            <tbody>
              @for (row of rows(); track $index) { <tr>@for (cell of row; track $index) { <td>{{ cell }}</td> }</tr> }
            </tbody>
          </table>
        </div>
        @if (moreRows()) { <p class="faint" i18n="@@share.reportBlock.moreRows">… and {{ moreRows() }} more rows</p> }
      }
      @case ('annotations') {
        <ul class="ann">
          @for (a of $any(block()).items; track $index) { <li><span class="faint">{{ when(a.from) }}</span> {{ a.label }}</li> }
        </ul>
      }
      @default {
        @if (image(); as src) {
          <img class="snap" [src]="src" [alt]="label()" />
        } @else if (block().type === 'chart' || block().type === 'timeline') {
          <p class="faint" i18n="@@share.reportBlock.noSnapshot">No snapshot.</p>
        } @else {
          <p class="faint" i18n="@@share.reportBlock.unknown">A "{{ block().type }}" block (from a newer Debrief) — kept as it is.</p>
        }
      }
    }
    @if (block().type !== 'text' && block().type !== 'finding') {
      <input class="input caption" [attr.aria-label]="captionLabel()" placeholder="Caption…" i18n-placeholder="@@share.reportBlock.captionPlaceholder" [value]="$any(block()).caption ?? ''" (change)="patch({ caption: val($event) })" />
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    :host(:focus-visible) { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .head { display: flex; align-items: center; gap: 4px; }
    .kind { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.04em; color: var(--muted); }
    .spacer { flex: 1; }
    .text { width: 100%; box-sizing: border-box; resize: vertical; font-family: var(--font-mono); }
    .md { padding: 4px 2px; }
    .ids { display: flex; flex-wrap: wrap; gap: 4px; list-style: none; margin: 0; padding: 0; }
    .link { padding: 0 6px; font-family: var(--font-mono); }
    .finding, .detail, .q { margin: 0; }
    .sev { text-transform: uppercase; font-size: var(--fs-xs); }
    .sev.critical { color: var(--danger-text); }
    .sev.warning { color: var(--warn-text); }
    .table-wrap { overflow: auto; max-height: 240px; }
    table { border-collapse: collapse; font-size: var(--fs-sm); }
    th, td { border: 1px solid var(--border-2); padding: 2px 6px; text-align: left; }
    .ann { margin: 0; padding-left: 18px; font-size: var(--fs-sm); }
    .snap { max-width: 100%; height: auto; background: var(--bg-1); border-radius: 4px; }
    .caption { width: 100%; box-sizing: border-box; }
  `,
})
export class ReportBlockView {
  readonly block = input.required<ReportBlock>();
  readonly first = input(false);
  readonly last = input(false);
  /** Emitted after a keyboard move so the view can keep focus on the block. */
  readonly moved = output<string>();

  protected readonly reports = inject(ReportStore);
  protected readonly ws = inject(WorkspaceStore);
  private readonly sanitizer = inject(DomSanitizer);

  protected readonly kindLabel = computed(() => LABELS[this.block().type] ?? this.block().type);
  protected readonly label = computed(() => {
    const b = this.block();
    const extra = b.type === 'chart' ? b.title : b.type === 'finding' ? b.title : 'caption' in b ? b.caption : '';
    const kind = this.kindLabel();
    return extra ? $localize`:@@share.reportBlock.labelExtra:${kind}:kind: block: ${extra}:extra:` : $localize`:@@share.reportBlock.label:${kind}:kind: block`;
  });
  protected readonly moveUpLabel = computed(() => $localize`:@@share.reportBlock.moveUpAria:Move up: ${this.label()}:label:`);
  protected readonly moveDownLabel = computed(() => $localize`:@@share.reportBlock.moveDownAria:Move down: ${this.label()}:label:`);
  protected readonly removeLabel = computed(() => $localize`:@@share.reportBlock.removeAria:Remove: ${this.label()}:label:`);
  protected readonly textLabel = computed(() => $localize`:@@share.reportBlock.textAria:Text of ${this.label()}:label:`);
  protected readonly captionLabel = computed(() => $localize`:@@share.reportBlock.captionAria:Caption of ${this.label()}:label:`);
  protected readonly image = computed<SafeUrl | null>(() => {
    const b = this.block();
    const svg = b.type === 'chart' || b.type === 'timeline' ? b.svg : null;
    // An <img> never runs scripts or loads anything an SVG references; the engine also sanitises it on export.
    return svg ? this.sanitizer.bypassSecurityTrustUrl(svgImageUrl(svg)) : null;
  });
  protected readonly rows = computed(() => {
    const b = this.block();
    return b.type === 'table' ? b.rows.slice(0, 50).map((r) => r.map((c) => valueToString(c))) : [];
  });
  protected readonly moreRows = computed(() => {
    const b = this.block();
    return b.type === 'table' ? Math.max(0, b.rows.length - 50) : 0;
  });

  protected showEventLabel(id: number): string {
    return $localize`:@@share.reportBlock.showEventAria:Show event ${id}:id:`;
  }

  protected severityLabel(s: string): string {
    return SEVERITY_LABELS[s] ?? s;
  }

  protected when(ts: number): string {
    return formatFull(ts, this.ws.timezone());
  }

  protected patch(p: Record<string, unknown>): void {
    this.reports.updateBlock(this.block().id, p);
  }

  protected move(delta: -1 | 1): void {
    this.reports.moveBlock(this.block().id, delta);
    this.moved.emit(this.block().id);
  }

  protected key(e: KeyboardEvent): void {
    if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    if (e.target !== e.currentTarget) return;
    e.preventDefault();
    this.move(e.key === 'ArrowUp' ? -1 : 1);
  }

  protected val = (e: Event) => (e.target as HTMLInputElement | HTMLTextAreaElement).value;
}
