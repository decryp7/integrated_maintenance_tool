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
import { TimelineZoom } from '../timeline/zoom';

/** Fields that tie events of one request/operation together, in order of preference. */
const CORRELATION_FIELDS = ['CorrelationId', 'RequestId', 'TraceId', 'OperationId', 'ActivityId', 'ConnectionId'];
const CONTEXT_MS = 2 * 60 * 1000;

@Component({
  selector: 'imt-inspector',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkerIcon, NgComponentOutlet],
  host: { '(document:selectionchange)': 'onSelectionChange()' },
  template: `
    <div class="head">
      <span class="section-title">Event</span>
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
          <div class="mono ts">{{ fullTime() }}</div>
          <div class="mono inst">{{ r.group }}</div>
          <div class="message">{{ r.message }}</div>
          @if (r.template && r.template !== r.message) {
            <div class="mono template">
              @for (part of templateParts(); track $index) {
                @if (part.hole) { <span class="hole">{{ part.text }}</span> } @else { {{ part.text }} }
              }
            </div>
          }
          <div class="actions">
            @if (r.ts !== null) {
              <button class="btn" (click)="showContext()">Show ±2 min</button>
            }
            @if (correlation(); as c) {
              <button class="btn" (click)="sameAs(c.field, c.value)">Same {{ c.field }}</button>
            }
            <button class="btn" (click)="copyJson()">{{ copied() ? 'Copied' : 'Copy JSON' }}</button>
            <button class="btn" [disabled]="!selection()" title="Select text in the message first" (click)="mask()">Mask…</button>
            <button class="btn" [disabled]="!selection()" title="Select a value in the message first" (click)="extract()">Extract value…</button>
            <button class="btn" aria-haspopup="menu" title="Drop, mark or filter events like this" (click)="openActions($event)">More actions…</button>
          </div>
        </div>

        <div class="section-title sub">Properties</div>
        <div class="props">
          @for (p of props(); track p.key) {
            <div class="prop" (contextmenu)="openPropMenu($event, p.key, p.value)" (keydown)="onPropKey($event, p.key, p.value)">
              <span class="mono k ellipsis" [title]="p.key">{{ p.key }}</span>
              <span class="mono v" [title]="p.display">{{ p.display }}</span>
              <span class="pa">
                <button class="icon-btn" [attr.aria-label]="'Filter ' + p.key + ' equals ' + p.value" title="Only this value" (click)="filter(p.key, 'eq', p.value)"><imt-icon name="plus" [size]="12" /></button>
                <button class="icon-btn" [attr.aria-label]="'Filter ' + p.key + ' not equal ' + p.value" title="Exclude this value" (click)="filter(p.key, 'ne', p.value)"><imt-icon name="minus" [size]="12" /></button>
                <button class="icon-btn" aria-haspopup="menu" [attr.aria-label]="'More actions for ' + p.key" title="More actions" (click)="openPropMenu($event, p.key, p.value)"><imt-icon name="drag" [size]="12" /></button>
              </span>
            </div>
          } @empty {
            <div class="faint pad">No properties.</div>
          }
        </div>

        @for (section of sections(); track section.id) {
          <div class="section-title sub">{{ section.title }}</div>
          <ng-container *ngComponentOutlet="section.component; inputs: { record: r }" />
        }

        @if (r.exception) {
          <div class="section-title sub">Exception</div>
          <pre class="mono exc">{{ r.exception }}</pre>
        }

        <div class="src faint">Source <span class="mono">{{ r.sourceName }}:{{ r.line }}</span></div>
      </div>
    } @else {
      <div class="empty-state">
        <div>Select an event in the grid or click a marker on the timeline.</div>
        <div class="faint">↑ / ↓ move through events.</div>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--bg-2); min-height: 0; }
    .head { height: 34px; flex: none; display: flex; align-items: center; gap: 8px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--border); font-size: 12px; }
    .spacer { flex: 1; }
    .scroll { flex: 1; min-height: 0; overflow: auto; }
    .top { padding: 14px; display: flex; flex-direction: column; gap: 8px; border-bottom: 1px solid var(--border); }
    .badges { display: flex; gap: 6px; flex-wrap: wrap; }
    .badge-lvl, .badge-mk { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 8px; border-radius: 4px; background: var(--bg-active); border: 1px solid var(--border-3); font-size: 12px; }
    .ts { font-size: 12px; color: var(--muted); }
    .inst { font-size: 12px; color: var(--text-2); }
    .message { font-size: 14px; line-height: 1.45; color: var(--text); white-space: pre-wrap; word-break: break-word; }
    .template { font-size: 11px; color: var(--faint); line-height: 1.5; word-break: break-word; }
    .hole { color: var(--chip-text); }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .sub { padding: 12px 14px 6px; }
    .props { display: flex; flex-direction: column; }
    .prop { display: grid; grid-template-columns: 118px minmax(0, 1fr) 72px; column-gap: 8px; align-items: start; padding: 3px 8px 3px 14px; }
    .prop:hover { background: var(--bg-hover); }
    .k { font-size: 11px; color: var(--muted); padding-top: 3px; }
    .v { font-size: 11px; color: var(--text); word-break: break-all; padding-top: 3px; }
    .pa { display: flex; gap: 2px; visibility: hidden; }
    .pa .icon-btn { width: 22px; height: 22px; }
    .prop:hover .pa, .pa:focus-within { visibility: visible; }
    .exc { margin: 0 14px; padding: 10px; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-size: 10.5px; line-height: 1.55; color: var(--text-2); white-space: pre; overflow: auto; max-height: 320px; }
    .src { padding: 12px 14px; font-size: 12px; }
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
  protected readonly rec = this.store.selected;
  protected readonly copied = signal(false);

  protected readonly levelVar = computed(() => {
    const r = this.rec();
    return r ? LEVEL_VARS[r.level] : '';
  });
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
      this.store.setError('Clipboard is not available.');
    }
  }
}
