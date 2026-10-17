import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { parseMarkdown } from './markdown';

/** Renders the report Markdown subset as plain DOM (text nodes only, never HTML from the text). */
@Component({
  selector: 'imt-markdown-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ng-template #inl let-items>
      @for (i of $any(items); track $index) {
        @switch (i.style) {
          @case ('strong') { <strong>{{ i.text }}</strong> }
          @case ('em') { <em>{{ i.text }}</em> }
          @case ('code') { <code>{{ i.text }}</code> }
          @case ('link') { <a [href]="i.url" target="_blank" rel="noopener noreferrer">{{ i.text }}</a> }
          @default { {{ i.text }} }
        }
      }
    </ng-template>
    @for (b of blocks(); track $index) {
      @switch (b.kind) {
        @case ('heading') { <p class="h" [class]="'h' + b.level" role="heading" [attr.aria-level]="b.level + 2"><ng-container *ngTemplateOutlet="inl; context: { $implicit: b.inlines }" /></p> }
        @case ('paragraph') { <p><ng-container *ngTemplateOutlet="inl; context: { $implicit: b.inlines }" /></p> }
        @case ('quote') { <blockquote><ng-container *ngTemplateOutlet="inl; context: { $implicit: b.inlines }" /></blockquote> }
        @case ('list') {
          @if (b.ordered) {
            <ol>@for (item of b.items; track $index) { <li><ng-container *ngTemplateOutlet="inl; context: { $implicit: item }" /></li> }</ol>
          } @else {
            <ul>@for (item of b.items; track $index) { <li><ng-container *ngTemplateOutlet="inl; context: { $implicit: item }" /></li> }</ul>
          }
        }
        @case ('code') { <pre><code>{{ b.text }}</code></pre> }
        @case ('rule') { <hr /> }
      }
    } @empty {
      <p class="faint">{{ placeholder() }}</p>
    }
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { display: block; font-size: 13px; line-height: 1.5; }
    p, ul, ol, blockquote, pre { margin: 0 0 6px; }
    .h { font-weight: 600; }
    .h1 { font-size: 16px; } .h2 { font-size: 15px; } .h3 { font-size: 14px; }
    blockquote { padding-left: 8px; border-left: 3px solid var(--border-3); color: var(--muted); }
    pre, code { font-family: var(--font-mono); background: var(--bg-3); border-radius: 3px; }
    pre { padding: 6px; white-space: pre-wrap; }
    a { color: var(--accent); }
  `,
})
export class MarkdownView {
  readonly text = input('');
  readonly placeholder = input('');
  protected readonly blocks = computed(() => parseMarkdown(this.text()));
}
