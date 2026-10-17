import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { AgentFieldSchema } from '../core/agent.types';
import { listToText, parseJsonObject, textToList } from './schedule-model';

let nextId = 0;

/** One field of the agent config, rendered from its schema (label, kind, limits, help, issues). */
@Component({
  selector: 'imt-schema-field',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let f = field();
    @switch (f.kind) {
      @case ('boolean') {
        <label class="check">
          <input type="checkbox" [id]="id" [checked]="!!value()" [attr.aria-describedby]="describedBy()" (change)="emit(checked($event))" />
          {{ f.label }}
        </label>
      }
      @default {
        <label class="lbl" [for]="id">{{ f.label }}@if (f.required) {<span aria-hidden="true"> *</span>}</label>
        @switch (f.kind) {
          @case ('select') {
            <select class="select" [id]="id" [value]="text()" [attr.aria-invalid]="!!problem()" [attr.aria-describedby]="describedBy()" (change)="emit(val($event))">
              @for (o of f.options ?? []; track o.value) { <option [value]="o.value" [selected]="o.value === text()">{{ o.label }}</option> }
            </select>
          }
          @case ('number') {
            <input
              class="input"
              type="number"
              [id]="id"
              [attr.min]="f.min ?? null"
              [attr.max]="f.max ?? null"
              [value]="text()"
              [attr.aria-invalid]="!!problem()"
              [attr.aria-describedby]="describedBy()"
              (input)="emitNumber(val($event))"
            />
          }
          @case ('list') {
            <textarea class="input mono" rows="2" [id]="id" [value]="text()" [attr.aria-invalid]="!!problem()" [attr.aria-describedby]="describedBy()" (input)="emit(toList(val($event)))"></textarea>
          }
          @case ('json') {
            <textarea class="input mono" rows="3" [id]="id" [value]="text()" placeholder='{"msinfo": true}' [attr.aria-invalid]="!!problem()" [attr.aria-describedby]="describedBy()" (input)="emitJson(val($event))"></textarea>
          }
          @default {
            <input
              class="input"
              [class.mono]="f.kind !== 'text'"
              [id]="id"
              [value]="text()"
              [attr.placeholder]="placeholder()"
              [attr.aria-required]="f.required"
              [attr.aria-invalid]="!!problem()"
              [attr.aria-describedby]="describedBy()"
              (input)="emit(val($event))"
            />
          }
        }
      }
    }
    @if (f.help) { <span class="help" [id]="id + '-help'">{{ f.help }}</span> }
    @if (problem(); as p) { <span class="problem" role="alert" [id]="id + '-err'">{{ p }}</span> }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 3px; min-width: 160px; flex: 1; }
    .lbl { font-size: 11px; color: var(--muted); }
    .check { display: inline-flex; gap: 6px; align-items: center; color: var(--text-2); margin-top: 16px; }
    .help { font-size: 11px; color: var(--muted); }
    .problem { font-size: 11px; color: var(--danger-text); }
    textarea { resize: vertical; }
  `,
})
export class SchemaField {
  readonly field = input.required<AgentFieldSchema>();
  readonly value = input<unknown>(undefined);
  /** Engine issues for this field. */
  readonly issues = input<string[] | undefined>(undefined);
  readonly valueChange = output<unknown>();

  protected readonly id = `imt-sf-${nextId++}`;
  /** A local problem (e.g. text that is not JSON yet); engine issues take precedence. */
  private readonly local = signal<string | null>(null);
  private readonly draftJson = signal<string | null>(null);

  protected readonly problem = computed(() => this.issues()?.join('; ') || this.local());
  protected readonly describedBy = computed(() => [this.field().help ? `${this.id}-help` : '', this.problem() ? `${this.id}-err` : ''].filter(Boolean).join(' ') || null);
  protected readonly placeholder = computed(() => {
    const f = this.field();
    if (f.kind === 'duration') return 'e.g. 15m';
    if (f.kind === 'cron') return '0 2 * * *';
    if (f.kind === 'query') return 'level >= Error';
    return null;
  });
  protected readonly text = computed(() => {
    const v = this.value();
    const f = this.field();
    if (f.kind === 'list') return listToText(v);
    if (f.kind === 'json') return this.draftJson() ?? (v && typeof v === 'object' ? JSON.stringify(v) : '');
    return v === undefined || v === null ? '' : String(v);
  });

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected toList = textToList;

  protected emit(v: unknown): void {
    this.valueChange.emit(v);
  }

  protected emitNumber(text: string): void {
    this.emit(text.trim() === '' ? undefined : Number(text));
  }

  protected emitJson(text: string): void {
    this.draftJson.set(text);
    const r = parseJsonObject(text);
    if ('error' in r) {
      this.local.set(r.error);
      return;
    }
    this.local.set(null);
    this.emit(r.value ?? undefined);
  }
}
