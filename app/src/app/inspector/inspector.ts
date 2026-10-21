import { NgComponentOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, linkedSignal, signal } from '@angular/core';
import { MessageSelection, RecordActions } from '../context/record-actions';
import { selectionWithin } from '../context/selection';
import { SettingsStore } from '../settings/settings.store';
import { FieldFormatters } from '../extensions/field-formatters';
import { InspectorSections } from '../extensions/inspector-sections';
import { RecordView } from '../core/api.types';
import { formatFull, valueToString } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { MarkerIcon } from '../shared/marker-icon';
import { ContextSection } from './context-section';
import { TimelineZoom } from '../timeline/zoom';
import { formatOffset } from '../skew/skew-format';
import { NextSteps } from '../ui/next-steps';
import { SourceColors } from '../sources/source-colors.store';
import { SourceFileActions } from '../sources/source-file-actions';
import { SourceSwatch } from '../sources/source-swatch';
import { FileAction } from '../core/api.types';

/** Fields that tie events of one request/operation together, in order of preference. */
const CORRELATION_FIELDS = ['CorrelationId', 'RequestId', 'TraceId', 'OperationId', 'ActivityId', 'ConnectionId'];
const CONTEXT_MS = 2 * 60 * 1000;

@Component({
  selector: 'imt-inspector',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkerIcon, NgComponentOutlet, ContextSection, NextSteps, SourceSwatch],
  host: { '(document:selectionchange)': 'onSelectionChange()' },
  template: `
    <div class="head">
      <span class="section-title" i18n="@@inspector.title">Event</span>
      @if (rec(); as r) { <span class="faint">#{{ r.id }}</span> }
      <span class="spacer"></span>
    </div>

    @if (rec(); as r) {
      <div class="scroll">
        <div class="top">
          <div class="badges">
            <span class="badge-lvl"><span class="dot" [style.background]="levelVar()"></span>{{ r.level }}</span>
            @for (m of r.markers; track m) {
              <span class="badge-mk"><imt-marker-icon [kind]="m" [color]="markerColor(m)" />{{ markerLabel(m) }}</span>
            }
          </div>
          <div class="mono ts imt-selectable">{{ fullTime() }}</div>
          @if (r.skewMs && r.ts !== null) {
            <div class="mono skew imt-selectable" data-testid="skew-note" i18n="@@inspector.skewNote">Shifted by {{ skewText(r.skewMs) //i18n(ph="offset") }} · original {{ originalTime(r.ts, r.skewMs) //i18n(ph="original") }}</div>
          }
          <div class="mono inst imt-selectable">{{ r.group }}</div>
          <div class="message imt-selectable">{{ r.message }}</div>
          @if (r.template && r.template !== r.message) {
            <div class="mono template imt-selectable">
              @for (part of templateParts(); track $index) {
                @if (part.hole) { <span class="hole">{{ part.text }}</span> } @else { {{ part.text }} }
              }
            </div>
          }
          <div class="actions">
            @if (r.ts !== null) {
              <button class="btn" (click)="showContext()" i18n="@@inspector.showContext">Show ±2 min</button>
            }
            @if (correlation(); as c) {
              <button class="btn" (click)="sameAs(c.field, c.value)" i18n="@@inspector.sameAs">Same {{ c.field //i18n(ph="field") }}</button>
            }
            <button class="btn" (click)="copyJson()">{{ copyJsonLabel() }}</button>
            <button class="btn" [disabled]="!selection()" title="Select text in the message first" i18n-title="@@inspector.mask.title" (click)="mask()" i18n="@@inspector.mask">Mask…</button>
            <button class="btn" [disabled]="!selection()" title="Select a value in the message first" i18n-title="@@inspector.extract.title" (click)="extract()" i18n="@@inspector.extract">Extract value…</button>
            <button class="btn" aria-haspopup="menu" title="Drop, mark or filter events like this" i18n-title="@@inspector.moreActions.title" (click)="openActions($event)" i18n="@@inspector.moreActions">More actions…</button>
          </div>
        </div>

        <div class="section-title sub" i18n="@@inspector.properties">Properties</div>
        <div class="props">
          @for (p of props(); track p.key) {
            <div class="prop" (contextmenu)="openPropMenu($event, p.key, p.value)" (keydown)="onPropKey($event, p.key, p.value)">
              <span class="mono k ellipsis" [title]="p.key">{{ p.key }}</span>
              <span class="mono v imt-selectable" [title]="p.display">{{ p.display }}</span>
              <span class="pa">
                <button class="icon-btn" [attr.aria-label]="filterEqLabel(p.key, p.value)" title="Only this value" i18n-title="@@inspector.prop.only" (click)="filter(p.key, 'eq', p.value)"><imt-icon name="plus" [size]="12" /></button>
                <button class="icon-btn" [attr.aria-label]="filterNeLabel(p.key, p.value)" title="Exclude this value" i18n-title="@@inspector.prop.exclude" (click)="filter(p.key, 'ne', p.value)"><imt-icon name="minus" [size]="12" /></button>
                <button class="icon-btn" aria-haspopup="menu" [attr.aria-label]="moreActionsLabel(p.key)" title="More actions" i18n-title="@@inspector.prop.moreActions" (click)="openPropMenu($event, p.key, p.value)"><imt-icon name="drag" [size]="12" /></button>
              </span>
            </div>
          } @empty {
            <div class="faint pad" i18n="@@inspector.noProperties">No properties.</div>
          }
        </div>

        <imt-context-section [record]="r" />

        @for (section of sections(); track section.id) {
          <div class="section-title sub">{{ section.title }}</div>
          <ng-container *ngComponentOutlet="section.component; inputs: { record: r }" />
        }

        @if (r.exception) {
          <div class="section-title sub" i18n="@@inspector.exception">Exception</div>
          <pre class="mono exc">{{ r.exception }}</pre>
        }

        <div class="src">
          <div class="src-line faint imt-selectable">
            @if (mark(); as m) {
              <imt-source-swatch [swatch]="m.swatch" [label]="m.label" />
            }
            <span i18n="@@inspector.source">Source <span class="mono">{{ r.sourceName }}:{{ r.line }}</span></span>
          </div>
          <div class="actions file-actions">
            @for (a of fileStates(); track a.action) {
              <button class="btn" type="button" [disabled]="!!a.reason" [attr.title]="a.reason" [attr.data-action]="a.action" (click)="openFile(a.action)">{{ a.label }}</button>
            }
          </div>
          @if (fileReason(); as why) {
            <div class="faint why" data-testid="file-reason">{{ why }}</div>
          }
        </div>
      </div>
    } @else {
      <div class="empty-state">
        @if (store.summary()?.sources?.length) {
          <div i18n="@@inspector.empty.select">Select an event in the grid or click a marker on the timeline.</div>
          <div class="faint" i18n="@@inspector.empty.keys">↑ / ↓ move through events.</div>
        } @else {
          <div i18n="@@inspector.empty.noFiles">Every property of an event shows here once you select it.</div>
          <imt-next-steps />
        }
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--bg-2); min-height: 0; }
    .head { height: 34px; flex: none; display: flex; align-items: center; gap: 8px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--border); font-size: var(--fs-sm); }
    .spacer { flex: 1; }
    .scroll { flex: 1; min-height: 0; overflow: auto; }
    .top { padding: 14px; display: flex; flex-direction: column; gap: 8px; border-bottom: 1px solid var(--border); }
    .badges { display: flex; gap: 6px; flex-wrap: wrap; }
    .badge-lvl, .badge-mk { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 8px; border-radius: 4px; background: var(--bg-active); border: 1px solid var(--border-3); font-size: var(--fs-sm); }
    .ts { font-size: var(--fs-sm); color: var(--muted); }
    .skew { font-size: var(--fs-xs); color: var(--warn-text); }
    .inst { font-size: var(--fs-sm); color: var(--text-2); }
    .message { font-size: var(--fs-lg); line-height: 1.45; color: var(--text); white-space: pre-wrap; word-break: break-word; }
    .template { font-size: var(--fs-xs); color: var(--faint); line-height: 1.5; word-break: break-word; }
    .hole { color: var(--chip-text); }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .sub { padding: 12px 14px 6px; }
    .props { display: flex; flex-direction: column; }
    .prop { display: grid; grid-template-columns: 118px minmax(0, 1fr) 72px; column-gap: 8px; align-items: start; padding: 3px 8px 3px 14px; }
    .prop:hover { background: var(--bg-hover); }
    .k { font-size: var(--fs-xs); color: var(--muted); padding-top: 3px; }
    .v { font-size: var(--fs-xs); color: var(--text); word-break: break-all; padding-top: 3px; }
    .pa { display: flex; gap: 2px; visibility: hidden; }
    .pa .icon-btn { width: 22px; height: 22px; }
    .prop:hover .pa, .pa:focus-within { visibility: visible; }
    .exc { margin: 0 14px; padding: 10px; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-size: var(--fs-2xs); line-height: 1.55; color: var(--text-2); white-space: pre; overflow: auto; max-height: 320px; }
    .src { padding: 12px 14px; font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 8px; }
    .src-line { display: flex; align-items: center; gap: 6px; }
    .why { font-size: var(--fs-xs); }
    .pad { padding: 0 14px; }
  `,
})
export class Inspector {
  private readonly actions = inject(RecordActions);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  /** Text currently selected inside the message (kept until the selection moves elsewhere in the message). */
  protected readonly selection = linkedSignal<RecordView | null, MessageSelection | null>({ source: () => this.rec(), computation: () => null });

  protected onSelectionChange(): void {
    const message = this.host.nativeElement.querySelector<HTMLElement>('.message');
    const sel = document.getSelection();
    if (!message || !sel || !sel.rangeCount || !message.contains(sel.anchorNode) || !message.contains(sel.focusNode)) return;
    this.selection.set(selectionWithin(message, sel.getRangeAt(0)));
  }

  protected mask(): void {
    const r = this.rec();
    const s = this.selection();
    if (r && s) this.actions.mask(r, s);
  }

  protected extract(): void {
    const r = this.rec();
    const s = this.selection();
    if (r && s) this.actions.extract(r, s);
  }

  protected openActions(e: MouseEvent): void {
    const r = this.rec();
    if (r) this.actions.openFor(e, r, { selection: this.selection() });
  }

  protected openPropMenu(e: MouseEvent, name: string, value: string): void {
    const r = this.rec();
    if (!r) return;
    e.preventDefault();
    this.actions.openFor(e, r, { field: { name, value }, selection: this.selection() });
  }

  protected onPropKey(e: KeyboardEvent, name: string, value: string): void {
    const r = this.rec();
    if (r && (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey))) {
      e.preventDefault();
      this.actions.openFor(e, r, { field: { name, value }, selection: this.selection() });
    }
  }
  protected readonly store = inject(WorkspaceStore);
  private readonly zoom = inject(TimelineZoom);
  private readonly files = inject(SourceFileActions);
  private readonly colors = inject(SourceColors);
  /** The line's source or instance colour (with its legend), as the grid stripe shows it. */
  protected readonly mark = computed(() => {
    const r = this.store.selected();
    return r ? this.colors.forRecord(r) : null;
  });
  /** Open file / Open at line / Show in folder for the event's file, with why one is unavailable. */
  protected readonly fileStates = computed(() => {
    const r = this.store.selected();
    return r ? this.files.states(this.files.sourceOf(r), r.line) : [];
  });
  protected readonly fileReason = computed(() => {
    const reasons = [...new Set(this.fileStates().map((a) => a.reason).filter((x): x is string => !!x))];
    return reasons.join(' · ') || null;
  });

  protected openFile(action: FileAction): void {
    const r = this.store.selected();
    const source = r ? this.files.sourceOf(r) : null;
    if (r && source) void this.files.run(source, action, r.line);
  }
  protected readonly rec = this.store.selected;
  protected readonly copied = signal(false);
  protected readonly copyJsonLabel = computed(() => (this.copied() ? $localize`:@@inspector.copied:Copied` : $localize`:@@inspector.copyJson:Copy JSON`));
  protected filterEqLabel(key: string, value: string): string {
    return $localize`:@@inspector.prop.filterEq:Filter ${key}:key: equals ${value}:value:`;
  }
  protected filterNeLabel(key: string, value: string): string {
    return $localize`:@@inspector.prop.filterNe:Filter ${key}:key: not equal ${value}:value:`;
  }
  protected moreActionsLabel(key: string): string {
    return $localize`:@@inspector.prop.moreActionsFor:More actions for ${key}:key:`;
  }

  protected readonly levelVar = computed(() => {
    const r = this.rec();
    return r ? LEVEL_VARS[r.level] : '';
  });
  protected readonly skewText = formatOffset;
  /** The timestamp before clock-skew correction. */
  protected originalTime(ts: number, skewMs: number): string {
    return formatFull(ts - skewMs, this.store.timezone(), this.settings.dateFormat());
  }
  protected readonly fullTime = computed(() => formatFull(this.rec()?.ts ?? null, this.store.timezone(), this.settings.dateFormat()));
  private readonly formatters = inject(FieldFormatters);
  private readonly settings = inject(SettingsStore);
  private readonly sectionRegistry = inject(InspectorSections);
  protected readonly sections = computed(() => {
    const r = this.rec();
    return r ? this.sectionRegistry.forRecord(r) : [];
  });
  /** `value` is the raw text used for filters; `display` is what plugins' formatters make of it. */
  protected readonly props = computed(() => {
    const r = this.rec();
    return Object.entries(r?.fields ?? {})
      .map(([key, v]) => ({ key, value: valueToString(v), display: this.formatters.format(key, v, r!) }))
      .sort((a, b) => a.key.localeCompare(b.key));
  });
  protected readonly correlation = computed(() => {
    const f = this.rec()?.fields ?? {};
    const field = CORRELATION_FIELDS.find((k) => f[k] !== undefined && f[k] !== '');
    return field ? { field, value: valueToString(f[field]) } : null;
  });
  protected readonly templateParts = computed(() => {
    const t = this.rec()?.template ?? '';
    const parts: { text: string; hole: boolean }[] = [];
    const re = /\{[^{}]+\}/g;
    let last = 0;
    for (let m = re.exec(t); m; m = re.exec(t)) {
      if (m.index > last) parts.push({ text: t.slice(last, m.index), hole: false });
      parts.push({ text: m[0], hole: true });
      last = m.index + m[0].length;
    }
    if (last < t.length) parts.push({ text: t.slice(last), hole: false });
    return parts;
  });

  protected markerColor(kind: string): string | null {
    return this.store.markerKindMap().get(kind)?.color ?? null;
  }
  protected markerLabel(kind: string): string {
    return this.store.markerKindMap().get(kind)?.label ?? kind;
  }

  protected filter(field: string, op: 'eq' | 'ne', value: string): void {
    this.store.addFilter({ field, op, value });
  }

  protected showContext(): void {
    const r = this.rec();
    if (r?.ts == null) return;
    const id = r.id;
    this.zoom.apply({ from: r.ts - CONTEXT_MS, to: r.ts + CONTEXT_MS });
    this.store.jumpTo(id);
  }

  protected sameAs(field: string, value: string): void {
    const id = this.rec()?.id;
    this.zoom.fit();
    this.store.addFilter({ field, op: 'eq', value });
    if (id !== undefined) this.store.jumpTo(id);
  }

  protected async copyJson(): Promise<void> {
    const r = this.rec();
    if (!r) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(r, null, 2));
      this.copied.set(true);
      setTimeout(() => this.copied.set(false), 1500);
    } catch {
      this.store.setError($localize`:@@inspector.clipboardUnavailable:Clipboard is not available.`);
    }
  }
}
