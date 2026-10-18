import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { RegexApi } from './regex-api';
import { ExplainNode } from '../../core/api.types';

const enc = new TextEncoder();
const dec = new TextDecoder();

/** Splits a pattern into before / highlighted / after using the engine's byte offsets. */
export function splitByBytes(pattern: string, start: number, end: number): [string, string, string] {
  const bytes = enc.encode(pattern);
  const a = Math.max(0, Math.min(start, bytes.length));
  const b = Math.max(a, Math.min(end, bytes.length));
  return [dec.decode(bytes.slice(0, a)), dec.decode(bytes.slice(a, b)), dec.decode(bytes.slice(b))];
}

/**
 * "Explain" button for any regex input: toggles an outline of what the pattern does; hovering or
 * focusing a line highlights the part of the pattern it explains. Reusable by other dialogs.
 *
 *   <imt-regex-assist [pattern]="regex()" />
 */
@Component({
  selector: 'imt-regex-assist',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button type="button" class="btn ghost small" [attr.aria-expanded]="open()" (click)="toggle()" i18n="@@markers.regexAssist.explain">Explain</button>
    @if (open()) {
      <div class="panel" role="region" aria-label="Regex explanation" i18n-aria-label="@@markers.regexAssist.region">
        @if (error(); as e) {
          <div class="err" role="alert">{{ e }}</div>
        } @else if (loading()) {
          <span class="faint" i18n="@@markers.regexAssist.explaining">Explaining…</span>
        } @else {
          <div class="pat mono" data-testid="regex-pattern">{{ parts()[0] }}<mark>{{ parts()[1] }}</mark>{{ parts()[2] }}</div>
          <ul class="outline">
            @for (n of nodes(); track $index) {
              <li tabindex="0" [style.padding-left.px]="n.depth * 14" (mouseenter)="active.set(n)" (focus)="active.set(n)" (mouseleave)="active.set(null)" (blur)="active.set(null)">{{ n.text }}</li>
            } @empty {
              <li class="faint" i18n="@@markers.regexAssist.nothing">Nothing to explain.</li>
            }
          </ul>
        }
      </div>
    }
  `,
  styles: `
    :host { display: inline-flex; flex-direction: column; align-items: flex-start; gap: 6px; }
    .small { font-size: 11px; padding: 1px 8px; }
    .panel { align-self: stretch; padding: 8px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--code-bg); font-size: 12px; max-height: 220px; overflow: auto; }
    .pat { word-break: break-all; margin-bottom: 6px; }
    mark { background: var(--accent-soft); color: var(--chip-text); border-radius: 2px; }
    .outline { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
    li { cursor: default; border-radius: 3px; }
    li:hover, li:focus { background: var(--bg-hover); outline: none; }
    .err { color: var(--danger-text); }
  `,
})
export class RegexAssist {
  private readonly api = inject(RegexApi);
  readonly pattern = input.required<string>();
  /** Outline to show instead of asking the engine (the infer dialog already has one). */
  readonly preset = input<ExplainNode[] | null>(null);

  protected readonly open = signal(false);
  protected readonly loading = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fetched = signal<ExplainNode[]>([]);
  protected readonly active = signal<ExplainNode | null>(null);
  protected readonly nodes = computed(() => this.preset() ?? this.fetched());
  protected readonly parts = computed(() => {
    const n = this.active();
    return n ? splitByBytes(this.pattern(), n.start, n.end) : ([this.pattern(), '', ''] as [string, string, string]);
  });
  private seq = 0;

  constructor() {
    effect(() => {
      this.pattern();
      if (untracked(() => this.open())) untracked(() => void this.refresh());
    });
  }

  protected async toggle(): Promise<void> {
    this.open.update((o) => !o);
    if (this.open()) await this.refresh();
  }

  /** Re-explains the current pattern (also after it was edited while open). */
  async refresh(): Promise<void> {
    if (this.preset()) return;
    const mine = ++this.seq;
    this.loading.set(true);
    this.error.set(null);
    try {
      const nodes = await this.api.explain(this.pattern());
      if (mine === this.seq) this.fetched.set(nodes);
    } catch (e) {
      if (mine === this.seq) this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      if (mine === this.seq) this.loading.set(false);
    }
  }
}
