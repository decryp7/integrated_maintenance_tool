import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { ExtractPreview, ExtractSample } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { valueToString } from '../core/format';
import { EXTRACT_FIELD_TYPES, ExtractField, ExtractRule } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { CAPTURE_VARS, OTHER_VAR } from '../shared/colors';
import { Icon } from '../shared/icon';
import { ConditionsEditor } from './conditions-editor';
import { ProfileDraft } from './profile-draft';

interface Segment {
  text: string;
  color: string | null;
  name: string | null;
}

/** Splits a sample into plain and captured segments (outermost captures win). */
function segments(sample: ExtractSample, colorOf: (name: string) => string): Segment[] {
  const spans = [...sample.spans].sort((a, b) => a.start - b.start || b.end - a.end);
  const out: Segment[] = [];
  let pos = 0;
  for (const s of spans) {
    if (s.start < pos) continue;
    if (s.start > pos) out.push({ text: sample.text.slice(pos, s.start), color: null, name: null });
    out.push({ text: sample.text.slice(s.start, s.end), color: colorOf(s.name), name: s.name });
    pos = s.end;
  }
  if (pos < sample.text.length) out.push({ text: sample.text.slice(pos), color: null, name: null });
  return out;
}

@Component({
  selector: 'imt-extractor-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConditionsEditor, Icon],
  template: `
    @if (rule(); as r) {
      <div class="grid2">
        <label class="lbl">Name<input class="input mono" [value]="r.name" (input)="rename($any($event.target).value)" /></label>
        <label class="lbl">Read from
          <input class="input mono" list="imt-profile-fields" [value]="r.from ?? '@message'" (input)="patch({ from: $any($event.target).value })" />
        </label>
      </div>

      <div class="lbl">Only when</div>
      <imt-conditions-editor [conditions]="r.when ?? []" (changed)="patch({ when: $event.length ? $event : undefined })" />

      <label class="lbl"><span>Pattern — named groups become fields: <span class="mono">(?P&lt;name&gt;…)</span></span>
        <textarea class="input mono pattern" rows="3" spellcheck="false" [value]="r.regex" (input)="patch({ regex: $any($event.target).value })"></textarea>
      </label>

      <div class="outputs-head">
        <span class="lbl">Outputs</span>
        <button class="btn ghost" (click)="detectOutputs()" title="Add an output for every named group in the pattern">Detect from pattern</button>
      </div>
      <div class="outputs">
        <span class="th">Field</span><span class="th">Type</span><span class="th">Unit</span><span class="th">Unit from group</span><span class="th">Convert to</span><span></span>
        @for (o of outputs(); track o.name) {
          <span class="mono cap" [style.background]="captureBg(o.name)">{{ o.name }}</span>
          <select class="select" [value]="o.spec.type" [attr.aria-label]="o.name + ' type'" (change)="patchOutput(o.name, { type: $any($event.target).value })">
            @for (t of types; track t) { <option [value]="t" [selected]="t === o.spec.type">{{ t }}</option> }
          </select>
          <input class="input mono" [value]="o.spec.unit ?? ''" placeholder="%, ms, MB" [attr.aria-label]="o.name + ' unit'" (input)="patchOutput(o.name, { unit: $any($event.target).value || undefined })" />
          <input class="input mono" [value]="o.spec.unit_from ?? ''" placeholder="group" [attr.aria-label]="o.name + ' unit from'" (input)="patchOutput(o.name, { unit_from: $any($event.target).value || undefined })" />
          <input class="input mono" [value]="o.spec.to ?? ''" placeholder="MB, ms, s" [attr.aria-label]="o.name + ' convert to'" (input)="patchOutput(o.name, { to: $any($event.target).value || undefined })" />
          <button class="icon-btn" [attr.aria-label]="'Remove output ' + o.name" (click)="removeOutput(o.name)" [attr.title]="'Remove output ' + o.name"><imt-icon name="close" [size]="12" /></button>
        } @empty {
          <span class="faint span-all">No outputs yet. Add named groups to the pattern, then "Detect from pattern".</span>
        }
      </div>

      <div class="preview-head">
        <span class="lbl">Live preview · current filters</span>
        @if (preview(); as p) {
          @if (p.error) {
            <span class="err">{{ p.error }}</span>
          } @else {
            <span class="stat" [class.ok]="p.failed === 0">{{ p.matches }} matches · {{ p.failed }} failed · {{ p.candidates }} candidates</span>
          }
        }
        @if (loading()) { <span class="spinner" aria-label="Loading"></span> }
      </div>
      <div class="samples mono">
        @for (s of samples(); track s.recordId) {
          <div class="sample" [title]="s.values">
            @for (seg of s.segs; track $index) {
              @if (seg.color) { <span class="hl" [style.background]="seg.color" [title]="seg.name">{{ seg.text }}</span> } @else { <span>{{ seg.text }}</span> }
            }
          </div>
        } @empty {
          <div class="faint">{{ preview() ? 'No matching events in the current results.' : 'Waiting for preview…' }}</div>
        }
      </div>
      @if (firstValues().length) {
        <div class="values">
          @for (v of firstValues(); track v.k) {
            <span class="mono k">{{ v.k }}</span><span class="mono">{{ v.v }}</span>
          }
        </div>
      }
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
    .grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--muted); }
    .pattern { font-size: 12px; line-height: 1.6; }
    .outputs-head, .preview-head { display: flex; align-items: center; gap: 10px; }
    .outputs { display: grid; grid-template-columns: 110px 100px 1fr 1fr 1fr 26px; gap: 4px; align-items: center; }
    .th { font-size: 11px; color: var(--faint); }
    .cap { padding: 2px 6px; border-radius: 3px; width: fit-content; font-size: 12px; color: var(--text); }
    .span-all { grid-column: 1 / -1; font-size: 12px; }
    .stat { font-size: 12px; color: var(--warn-text); }
    .stat.ok { color: var(--ok); }
    .err { color: var(--danger-text); font-size: 12px; }
    .samples { padding: 8px 10px; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-size: 11px; line-height: 1.8; color: var(--muted); overflow-x: auto; max-height: 260px; }
    .sample { white-space: pre; }
    .hl { color: var(--text); border-radius: 2px; }
    .values { display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; font-size: 11px; }
    .values .k { color: var(--muted); }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
  `,
})
export class ExtractorEditor {
  readonly index = input.required<number>();

  private readonly draft = inject(ProfileDraft);
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  protected readonly types = EXTRACT_FIELD_TYPES;
  protected readonly preview = signal<ExtractPreview | null>(null);
  protected readonly loading = signal(false);

  protected readonly rule = computed<ExtractRule | null>(() => this.draft.model().extract?.[this.index()] ?? null);
  protected readonly outputs = computed(() => Object.entries(this.rule()?.fields ?? {}).map(([name, spec]) => ({ name, spec })));
  private readonly captureNames = computed(() => {
    const names = [...(this.rule()?.regex ?? '').matchAll(/\(\?P?<([A-Za-z_][A-Za-z0-9_]*)>/g)].map((m) => m[1]);
    return [...new Set(names)];
  });
  protected readonly samples = computed(() =>
    (this.preview()?.samples ?? []).map((s) => ({
      recordId: s.recordId,
      segs: segments(s, (n) => this.captureColor(n)),
      values: Object.entries(s.values)
        .map(([k, v]) => `${k} = ${valueToString(v)}`)
        .join('\n'),
    })),
  );
  protected readonly firstValues = computed(() =>
    Object.entries(this.preview()?.samples[0]?.values ?? {}).map(([k, v]) => ({ k, v: valueToString(v) })),
  );

  constructor() {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let seq = 0;
    effect(() => {
      const rule = this.rule();
      const toml = this.draft.text();
      const query = this.store.effectiveQuery();
      const parseOk = !this.draft.parseError();
      clearTimeout(timer);
      if (!rule || !parseOk) return;
      const mine = ++seq;
      untracked(() => this.loading.set(true));
      timer = setTimeout(async () => {
        try {
          const p = await this.backend.previewExtractor({ toml, extractor: rule.name, query, limit: 12 });
          if (mine === seq) this.preview.set(p);
        } catch (e) {
          if (mine === seq) this.preview.set({ candidates: 0, matches: 0, failed: 0, samples: [], error: String(e) });
        } finally {
          if (mine === seq) this.loading.set(false);
        }
      }, 350);
    });
  }

  private captureColor(name: string): string {
    const i = this.captureNames().indexOf(name);
    const base = i >= 0 && i < CAPTURE_VARS.length ? CAPTURE_VARS[i] : OTHER_VAR;
    return `color-mix(in srgb, ${base} 28%, transparent)`;
  }

  protected captureBg(name: string): string {
    return this.captureColor(name);
  }

  protected patch(p: Partial<ExtractRule>): void {
    const i = this.index();
    this.draft.update((m) => {
      const rule = m.extract?.[i];
      if (!rule) return;
      for (const [k, v] of Object.entries(p)) {
        if (v === undefined) delete rule[k];
        else rule[k] = v;
      }
    });
  }

  protected rename(name: string): void {
    this.patch({ name });
  }

  protected patchOutput(name: string, p: Partial<ExtractField>): void {
    const i = this.index();
    this.draft.update((m) => {
      const spec = m.extract?.[i]?.fields?.[name];
      if (!spec) return;
      for (const [k, v] of Object.entries(p)) {
        if (v === undefined) delete spec[k];
        else spec[k] = v;
      }
    });
  }

  protected removeOutput(name: string): void {
    const i = this.index();
    this.draft.update((m) => {
      const fields = m.extract?.[i]?.fields;
      if (fields) delete fields[name];
    });
  }

  protected detectOutputs(): void {
    const i = this.index();
    const names = this.captureNames();
    this.draft.update((m) => {
      const rule = m.extract?.[i];
      if (!rule) return;
      rule.fields ??= {};
      const unitGroups = new Set(Object.values(rule.fields).map((f) => f.unit_from).filter(Boolean));
      for (const n of names) {
        if (rule.fields[n] || unitGroups.has(n) || /_unit$/.test(n)) continue;
        rule.fields[n] = { type: 'string' };
      }
    });
  }
}
