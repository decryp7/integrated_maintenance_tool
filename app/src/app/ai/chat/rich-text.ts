import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { Citation } from '../ai.types';
import { Block, IncrementalMarkdown, Inline } from './markdown';

/**
 * Renders a model answer from the safe Markdown subset with text bindings only (no `innerHTML`), with
 * evidence as links: records jump to the grid, queries apply, web pages open in the browser after a
 * confirmation.
 */
@Component({
  selector: 'debrief-ai-rich-text',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ng-template #line let-inlines>
      @for (i of asInlines(inlines); track $index) {
        @switch (i.kind) {
          @case ('text') {<span>{{ i.text }}</span>}
          @case ('code') {<code class="mono">{{ i.text }}</code>}
          @case ('strong') {<strong>{{ i.text }}</strong>}
          @case ('em') {<em>{{ i.text }}</em>}
          @case ('record') {<button type="button" class="cite rec" [class.unverified]="!rec(i).verified" [attr.title]="recTitle(i)" (click)="record.emit(rec(i).id)">{{ i.text }}</button>}
          @case ('query') {<button type="button" class="cite q mono" [attr.title]="queryTitle(i)" (click)="query.emit(q(i).ql)">{{ i.text }}</button>}
          @case ('web') {<button type="button" class="cite web" [attr.title]="webTitle(i)" (click)="link.emit({ url: web(i).url, title: web(i).title })">{{ i.text }}</button>}
        }
      }
    </ng-template>
    @for (b of blocks(); track $index) {
      @switch (b.kind) {
        @case ('p') {<p><ng-container *ngTemplateOutlet="line; context: { $implicit: inl(b) }" /></p>}
        @case ('h') {<p class="h" role="heading" [attr.aria-level]="hl(b) + 2"><ng-container *ngTemplateOutlet="line; context: { $implicit: inl(b) }" /></p>}
        @case ('ul') {<ul>@for (item of items(b); track $index) {<li><ng-container *ngTemplateOutlet="line; context: { $implicit: item }" /></li>}</ul>}
        @case ('ol') {<ol>@for (item of items(b); track $index) {<li><ng-container *ngTemplateOutlet="line; context: { $implicit: item }" /></li>}</ol>}
        @case ('code') {<pre class="mono"><code>{{ codeText(b) }}</code></pre>}
      }
    }
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { display: block; line-height: 1.5; overflow-wrap: anywhere; }
    p { margin: 0 0 8px; }
    p:last-child { margin-bottom: 0; }
    .h { font-weight: 600; color: var(--text); }
    ul, ol { margin: 0 0 8px; padding-left: 20px; }
    li { margin: 2px 0; }
    code { background: var(--code-bg); padding: 0 3px; border-radius: 3px; font-size: 0.92em; }
    pre { background: var(--code-bg); padding: 8px; border-radius: 6px; overflow-x: auto; margin: 0 0 8px; font-size: var(--fs-sm); white-space: pre; }
    pre code { background: none; padding: 0; }
    .cite { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; text-underline-offset: 2px; }
    .cite:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; border-radius: 2px; }
    .cite.q { color: inherit; background: var(--code-bg); padding: 0 3px; border-radius: 3px; text-decoration: none; border-bottom: 1px dashed var(--accent); }
    .cite.unverified { color: var(--muted); text-decoration-style: dotted; }
  `,
})
export class AiRichText {
  readonly text = input.required<string>();
  readonly citations = input<readonly Citation[]>([]);
  readonly record = output<number>();
  readonly query = output<string>();
  readonly link = output<{ url: string; title: string }>();

  /** While an answer streams in, only its last open block is parsed again (closed blocks are kept). */
  private readonly markdown = new IncrementalMarkdown();
  protected readonly blocks = computed(() => this.markdown.parse(this.text(), this.citations()));

  protected asInlines(v: unknown): Inline[] {
    return v as Inline[];
  }
  protected inl(b: Block): Inline[] {
    return 'inlines' in b ? b.inlines : [];
  }
  protected hl(b: Block): number {
    return b.kind === 'h' ? b.level : 1;
  }
  protected items(b: Block): Inline[][] {
    return b.kind === 'ul' || b.kind === 'ol' ? b.items : [];
  }
  protected codeText(b: Block): string {
    return b.kind === 'code' ? b.text : '';
  }
  protected rec(i: Inline): { id: number; verified: boolean } {
    return i.kind === 'record' ? i : { id: 0, verified: false };
  }
  protected q(i: Inline): { ql: string } {
    return i.kind === 'query' ? i : { ql: '' };
  }
  protected recTitle(i: Inline): string {
    const r = this.rec(i);
    return r.verified
      ? $localize`:@@ai.richText.record.show:Show event ${r.id}:id: in the grid`
      : $localize`:@@ai.richText.record.unverified:Event ${r.id}:id: (not seen in a tool result: unverified)`;
  }
  protected queryTitle(i: Inline): string {
    return $localize`:@@ai.richText.query.apply:Apply the query ${this.q(i).ql}:query:`;
  }
  protected webTitle(i: Inline): string {
    return $localize`:@@ai.richText.web.open:Open ${this.web(i).title}:title: in the browser`;
  }
  protected web(i: Inline): { url: string; title: string } {
    return i.kind === 'web' ? i : { url: '', title: '' };
  }
}
