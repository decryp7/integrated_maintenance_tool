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
import { AiButton } from '../../ai/ai-button';
import { AiLauncher } from '../../ai/ai-launcher';
import { plural } from '../../i18n/locale';

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
  imports: [RegexAssist, AiButton],
  template: `
    <label class="lbl"><ng-container i18n="@@markers.infer.lines">Lines (one per line; edit to refine)</ng-container>
      <textarea class="input mono" rows="4" [value]="text()" (input)="text.set($any($event.target).value)"></textarea>
    </label>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    @if (result(); as r) {
      <label class="lbl"><ng-container i18n="@@markers.infer.pattern">Pattern (editable)</ng-container>
        <input class="input mono" [value]="pattern()" (input)="pattern.set($any($event.target).value)" />
      </label>
      <div><imt-ai-button label="Suggest with AI" i18n-label="@@markers.infer.suggestAi" hint="The local model proposes a pattern; the engine checks it against these lines" i18n-hint="@@markers.infer.suggestAiHint" testId="ai-suggest-regex" (pressed)="suggestWithAi()" /></div>
      <imt-regex-assist [pattern]="pattern()" [preset]="pattern() === r.pattern ? r.explanation : null" />
      <table class="groups" aria-label="Captured groups" i18n-aria-label="@@markers.infer.groups">
        <thead><tr><th scope="col" i18n="@@markers.infer.col.group">Group</th><th scope="col" i18n="@@markers.infer.col.kind">Kind</th><th scope="col" i18n="@@markers.infer.col.examples">Examples</th></tr></thead>
        <tbody>
          @for (g of r.groups; track g.name) {
            <tr>
              <td><input class="input mono" [value]="nameOf(g)" [attr.aria-label]="groupLabel(g.name)" (input)="rename(g.name, $any($event.target).value)" /></td>
              <td>{{ g.kind }}</td>
              <td class="mono">{{ g.examples.join(', ') }}</td>
            </tr>
          } @empty {
            <tr><td colspan="3" class="faint" i18n="@@markers.infer.noVariable">No variable parts: the lines are identical.</td></tr>
          }
        </tbody>
      </table>
      <ul class="lines" aria-label="Which lines match" i18n-aria-label="@@markers.infer.matchList">
        @for (l of inputLines(); track $index; let i = $index) {
          <li><span [class.ok]="r.matches[i]" [class.no]="!r.matches[i]">{{ r.matches[i] ? '✓' : '✗' }}</span><span class="sr">@if (r.matches[i]) { <ng-container i18n="@@markers.infer.matches">matches</ng-container> } @else { <ng-container i18n="@@markers.infer.noMatch">does not match</ng-container> }</span> <span class="mono">{{ l }}</span></li>
        }
      </ul>
      @if (invalidNames().length) { <div class="err" role="alert" i18n="@@markers.infer.invalidNames">Group names use letters, digits and _, not starting with a digit: {{ invalidNames().join(', ') }}.</div> }
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@markers.infer.close">Close</button>
        <button type="button" class="btn" (click)="filter()" i18n="@@markers.infer.filter">Filter</button>
        <button type="button" class="btn" [disabled]="!r.groups.length || invalidNames().length > 0 || saving()" (click)="extract()" i18n="@@markers.infer.extract">Extract fields…</button>
        <button type="button" class="btn primary" (click)="highlight()" i18n="@@markers.infer.highlight">Highlight these lines</button>
      </div>
    } @else if (!error()) {
      <div class="status" role="status" i18n="@@markers.infer.inferring">Inferring…</div>
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
  private readonly ai = inject(AiLauncher);

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

  /** "Suggest regex" with the local AI model: the accepted pattern replaces the inferred one. */
  protected async suggestWithAi(): Promise<void> {
    const p = await this.ai.suggestRegex(this.inputLines());
    if (p) this.pattern.set(p);
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
      this.error.set($localize`:@@markers.infer.needLine:Add at least one line.`);
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

  protected groupLabel(name: string): string {
    return $localize`:@@markers.infer.groupAria:Name of group ${name}:name:`;
  }

  /** The pattern with the user's group names applied. */
  protected finalPattern(): string {
    return renameGroups(this.pattern(), this.renames());
  }


  protected highlight(): void {
    this.ref.close();
    this.dialogs.open(MarkDialog, {
      title: $localize`:@@markers.infer.dialog.title:Highlight lines`,
      inputs: {
        initial: [{ field: '@message', op: 'matches', value: this.finalPattern() }],
        suggestedLabel: $localize`:@@markers.infer.dialog.label:Inferred pattern`,
        submitLabel: $localize`:@@markers.infer.dialog.submit:Add highlight`,
        hint: $localize`:@@markers.infer.dialog.hint:The inferred regex is a "matches" condition; edit it below and check the preview.`,
      },
      width: 760,
    });
  }

  protected filter(): void {
    this.store.patchQuery({ ql: `message ~ ${qlQuote(this.finalPattern())}` });
    this.store.setNotice($localize`:@@markers.infer.filtering:Filtering by the inferred pattern.`);
    this.ref.close();
  }

  protected async extract(): Promise<void> {
    const r = this.result();
    const firstGroup = r?.groups[0];
    if (!r || !firstGroup) return;
    this.saving.set(true);
    const fields: Record<string, ExtractField> = {};
    for (const g of r.groups) fields[this.nameOf(g)] = { type: fieldTypeFor(g.kind) };
    const first = this.nameOf(firstGroup);
    const rule: ExtractRule = { name: `extract-${first}`.toLowerCase(), from: '@message', regex: this.finalPattern(), fields };
    const res = await this.profile.commit((model) => addExtractRule(model, rule));
    this.saving.set(false);
    if (!res.ok) {
      this.error.set(
        res.failure.line
          ? $localize`:@@markers.infer.failureLine:${res.failure.message}:message: (profile line ${res.failure.line}:line:)`
          : res.failure.message,
      );
      return;
    }
    const count = r.groups.length;
    this.store.setNotice(
      plural(count, {
        one: $localize`:@@markers.infer.extractorAdded.one:Added an extractor with ${count}:count: field; files were re-imported.`,
        other: $localize`:@@markers.infer.extractorAdded.other:Added an extractor with ${count}:count: fields; files were re-imported.`,
      }),
    );
    this.ref.close(true);
  }
}
