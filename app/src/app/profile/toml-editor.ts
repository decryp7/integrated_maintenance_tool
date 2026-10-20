import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, viewChild } from '@angular/core';
import { ProfileDraft } from './profile-draft';

/** Plain TOML editing with line numbers; the backend's validation error is pinned to its line. */
@Component({
  selector: 'imt-toml-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="editor">
      <div class="gutter mono" #gutter aria-hidden="true">
        @for (n of lineNumbers(); track n) {
          <div [class.err]="n === errorLine()">{{ n }}</div>
        }
      </div>
      <textarea
        #area
        class="mono"
        spellcheck="false"
        aria-label="Profile TOML" i18n-aria-label="@@profile.toml.label"
        [value]="draft.text()"
        (input)="draft.setText($any($event.target).value)"
        (scroll)="syncScroll()"
        (keydown.tab)="insertTab($event)"
      ></textarea>
    </div>
    @if (draft.problem(); as p) {
      <div class="problem" role="status">
        @if (p.line) { <button class="link mono" (click)="goToLine(p.line)" i18n="@@profile.toml.line">Line {{ p.line }}</button> }
        <span>{{ p.message }}</span>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .editor { flex: 1; min-height: 0; display: flex; background: var(--code-bg); }
    .gutter { width: 52px; flex: none; overflow: hidden; padding: 10px 8px 10px 0; text-align: right; color: var(--faint); font-size: var(--fs-sm); line-height: 19px; border-right: 1px solid var(--border); user-select: none; }
    .gutter .err { color: var(--lvl-error); font-weight: 600; background: color-mix(in srgb, var(--lvl-error) 15%, transparent); }
    textarea { flex: 1; resize: none; border: 0; outline: none; padding: 10px 12px; background: transparent; color: var(--text); font-size: var(--fs-sm); line-height: 19px; white-space: pre; overflow: auto; tab-size: 2; }
    .problem { flex: none; display: flex; gap: 10px; align-items: center; padding: 8px 14px; border-top: 1px solid var(--lvl-error); color: var(--danger-text); background: var(--bg-2); font-size: var(--fs-sm); }
    .link { border: 0; background: transparent; color: inherit; text-decoration: underline; cursor: pointer; padding: 0; font-size: var(--fs-sm); }
  `,
})
export class TomlEditor {
  protected readonly draft = inject(ProfileDraft);
  private readonly gutter = viewChild.required<ElementRef<HTMLElement>>('gutter');
  private readonly area = viewChild.required<ElementRef<HTMLTextAreaElement>>('area');

  protected readonly lineNumbers = computed(() => {
    const n = this.draft.text().split('\n').length;
    return Array.from({ length: n }, (_, i) => i + 1);
  });
  protected readonly errorLine = computed(() => this.draft.problem()?.line ?? null);

  protected syncScroll(): void {
    this.gutter().nativeElement.scrollTop = this.area().nativeElement.scrollTop;
  }

  protected insertTab(e: Event): void {
    e.preventDefault();
    const ta = this.area().nativeElement;
    const { selectionStart: s, selectionEnd: end, value } = ta;
    ta.value = value.slice(0, s) + '  ' + value.slice(end);
    ta.selectionStart = ta.selectionEnd = s + 2;
    this.draft.setText(ta.value);
  }

  protected goToLine(line: number): void {
    const ta = this.area().nativeElement;
    const lines = ta.value.split('\n');
    const pos = lines.slice(0, line - 1).reduce((a, l) => a + l.length + 1, 0);
    ta.focus();
    ta.setSelectionRange(pos, pos + (lines[line - 1]?.length ?? 0));
    ta.scrollTop = Math.max(0, (line - 5) * 19);
    this.syncScroll();
  }
}
