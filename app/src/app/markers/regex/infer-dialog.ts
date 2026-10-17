import { ChangeDetectionStrategy, Component, computed, effect, inject, input, linkedSignal, signal, untracked } from '@angular/core';
import { DialogService, DIALOG_REF } from '../../ui/dialog.service';
import { DIALOG_FORM_STYLES } from '../../context/dialog-styles';
import { addExtractRule } from '../../context/rule-builders';
import { MarkDialog } from '../../context/mark-dialog';
import { ExtractField, ExtractFieldType, ExtractRule } from '../../core/profile.model';
import { WorkspaceStore } from '../../core/workspace.store';
import { MarkerProfile } from '../marker-profile';
import { RegexApi } from './regex-api';
import { RegexAssist } from './regex-assist';
import { GroupKind, InferredGroup, InferredRegex } from '../../core/api.types';
import { qlQuote } from '../../analysis/ql-text';

/** Extract field type for an inferred group kind. */
export function fieldTypeFor(kind: GroupKind): ExtractFieldType {
  return kind === 'number' ? 'integer' : kind === 'decimal' ? 'number' : 'string';
}

/** Renames capture groups in a pattern (`(?P<old>` to `(?P<new>`). */
export function renameGroups(pattern: string, renames: Record<string, string>): string {
  return pattern.replace(/\(\?P?<(\w+)>/g, (m, name: string) => (renames[name] && renames[name] !== name ? `(?P<${renames[name]}>` : m));
}

const VALID_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** "Infer pattern from lines": one regex for several lines, explained, with the group table and follow-up actions. */
@Component({
  selector: 'imt-infer-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RegexAssist],
  template: `
    <label class="lbl">Lines (one per line; edit to refine)
      <textarea class="input mono" rows="4" [value]="text()" (input)="text.set($any($event.target).value)"></textarea>
    </label>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    @if (result(); as r) {
      <label class="lbl">Pattern (editable)
        <input class="input mono" [value]="pattern()" (input)="pattern.set($any($event.target).value)" />
      </label>
      <imt-regex-assist [pattern]="pattern()" [preset]="pattern() === r.pattern ? r.explanation : null" />
      <table class="groups" aria-label="Captured groups">
        <thead><tr><th scope="col">Group</th><th scope="col">Kind</th><th scope="col">Examples</th></tr></thead>
        <tbody>
          @for (g of r.groups; track g.name) {
            <tr>
              <td><input class="input mono" [value]="nameOf(g)" [attr.aria-label]="'Name of group ' + g.name" (input)="rename(g.name, $any($event.target).value)" /></td>
              <td>{{ g.kind }}</td>
              <td class="mono">{{ g.examples.join(', ') }}</td>
            </tr>
          } @empty {
            <tr><td colspan="3" class="faint">No variable parts: the lines are identical.</td></tr>
          }
        </tbody>
      </table>
      <ul class="lines" aria-label="Which lines match">
        @for (l of inputLines(); track $index; let i = $index) {
          <li><span [class.ok]="r.matches[i]" [class.no]="!r.matches[i]">{{ r.matches[i] ? '✓' : '✗' }}</span><span class="sr">{{ r.matches[i] ? 'matches' : 'does not match' }}</span> <span class="mono">{{ l }}</span></li>
        }
      </ul>
      @if (invalidNames().length) { <div class="err" role="alert">Group names use letters, digits and _, not starting with a digit: {{ invalidNames().join(', ') }}.</div> }
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()">Close</button>
        <button type="button" class="btn" (click)="filter()">Filter</button>
        <button type="button" class="btn" [disabled]="!r.groups.length || invalidNames().length > 0 || saving()" (click)="extract()">Extract fields…</button>
        <button type="button" class="btn primary" (click)="highlight()">Highlight these lines</button>
      </div>
    } @else if (!error()) {
      <div class="status" role="status">Inferring…</div>
    }
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
    textarea { resize: vertical; }
    .groups { width: 100%; border-collapse: collapse; font-size: 12px; }
    .groups th, .groups td { text-align: left; padding: 3px 6px; border-bottom: 1px solid var(--border); }
    .lines { list-style: none; margin: 0; padding: 0; font-size: 12px; max-height: 120px; overflow: auto; }
    .ok { color: var(--success-text, #3fbf7f); }
    .no { color: var(--danger-text); }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  `,
  ],
})
export class InferDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly api = inject(RegexApi);
  private readonly dialogs = inject(DialogService);
  private readonly profile = inject(MarkerProfile);
  private readonly store = inject(WorkspaceStore);

  readonly lines = input<string[]>([]);
  protected readonly text = linkedSignal(() => this.lines().join('\n'));
  protected readonly result = signal<InferredRegex | null>(null);
  protected readonly error = signal<string | null>(null);
  protected readonly pattern = signal('');
  protected readonly renames = signal<Record<string, string>>({});
  protected readonly saving = signal(false);
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;

  protected readonly inputLines = computed(() => this.text().split('\n').filter((l) => l.trim()));
  protected readonly invalidNames = computed(() => (this.result()?.groups ?? []).map((g) => this.nameOf(g)).filter((n) => !VALID_NAME.test(n)));

  constructor() {
    effect(() => {
      const lines = this.inputLines();
      clearTimeout(this.timer);
      this.timer = setTimeout(() => void this.run(lines), untracked(() => (this.result() ? 300 : 0)));
    });
  }

  protected nameOf(g: InferredGroup): string {
    return this.renames()[g.name] ?? g.name;
  }

  protected rename(name: string, value: string): void {
    this.renames.update((r) => ({ ...r, [name]: value }));
  }

  private async run(lines: string[]): Promise<void> {
    if (!lines.length) {
      this.result.set(null);
      this.error.set('Add at least one line.');
      return;
    }
    const mine = ++this.seq;
    try {
      const r = await this.api.infer(lines);
      if (mine !== this.seq) return;
      this.error.set(null);
      this.renames.set({});
      this.result.set(r);
      this.pattern.set(r.pattern);
    } catch (e) {
      if (mine === this.seq) this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  /** The pattern with the user's group names applied. */
  protected finalPattern(): string {
    return renameGroups(this.pattern(), this.renames());
  }


  protected highlight(): void {
    this.ref.close();
    this.dialogs.open(MarkDialog, {
      title: 'Highlight lines',
      inputs: {
        initial: [{ field: '@message', op: 'matches', value: this.finalPattern() }],
        suggestedLabel: 'Inferred pattern',
        submitLabel: 'Add highlight',
        hint: 'The inferred regex is a "matches" condition; edit it below and check the preview.',
      },
      width: 760,
    });
  }

  protected filter(): void {
    this.store.patchQuery({ ql: `message ~ ${qlQuote(this.finalPattern())}` });
    this.store.setNotice('Filtering by the inferred pattern.');
    this.ref.close();
  }

  protected async extract(): Promise<void> {
    const r = this.result();
    if (!r) return;
    this.saving.set(true);
    const fields: Record<string, ExtractField> = {};
    for (const g of r.groups) fields[this.nameOf(g)] = { type: fieldTypeFor(g.kind) };
    const first = this.nameOf(r.groups[0]);
    const rule: ExtractRule = { name: `extract-${first}`.toLowerCase(), from: '@message', regex: this.finalPattern(), fields };
    const res = await this.profile.commit((model) => addExtractRule(model, rule));
    this.saving.set(false);
    if (!res.ok) {
      this.error.set(res.failure.message + (res.failure.line ? ` (profile line ${res.failure.line})` : ''));
      return;
    }
    this.store.setNotice(`Added an extractor with ${r.groups.length} field${r.groups.length === 1 ? '' : 's'}; files were re-imported.`);
    this.ref.close(true);
  }
}
