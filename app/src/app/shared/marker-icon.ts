import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

const BUILT_IN = new Set(['start', 'stop', 'fatal', 'exception', 'error', 'warning']);

/** Shape + colour for a marker kind. Built-in kinds have distinct shapes so colour is never the only cue. */
@Component({
  selector: 'imt-marker-icon',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span class="mk" [class]="cls()" [style.background]="customColor()" [attr.aria-label]="label() || kind()" role="img"></span>`,
  styles: `
    :host { display: inline-flex; width: 12px; height: 12px; align-items: center; justify-content: center; flex: none; }
    .mk { display: block; width: 10px; height: 10px; }
    .mk-start { background: var(--mk-start); clip-path: polygon(10% 0, 100% 50%, 10% 100%); }
    .mk-stop { background: var(--mk-stop); width: 9px; height: 9px; border-radius: 1px; }
    .mk-exception { background: var(--mk-exception); width: 8px; height: 8px; transform: rotate(45deg); }
    .mk-error { background: var(--mk-error); border-radius: 50%; width: 9px; height: 9px; }
    .mk-warning { background: var(--mk-warning); clip-path: polygon(50% 4%, 100% 96%, 0 96%); }
    .mk-fatal { background: var(--mk-fatal); clip-path: polygon(20% 0, 50% 30%, 80% 0, 100% 20%, 70% 50%, 100% 80%, 80% 100%, 50% 70%, 20% 100%, 0 80%, 30% 50%, 0 20%); }
    .mk-custom { border-radius: 50%; width: 8px; height: 8px; box-shadow: 0 0 0 1.5px var(--bg-1), 0 0 0 2.5px currentColor; color: var(--muted); }
  `,
})
export class MarkerIcon {
  readonly kind = input.required<string>();
  readonly color = input<string | null>(null);
  readonly label = input<string>('');

  protected readonly isBuiltIn = computed(() => BUILT_IN.has(this.kind()));
  protected readonly cls = computed(() => `mk ${this.isBuiltIn() ? 'mk-' + this.kind() : 'mk-custom'}`);
  protected readonly customColor = computed(() => (this.isBuiltIn() ? null : (this.color() ?? 'var(--muted)')));
}
