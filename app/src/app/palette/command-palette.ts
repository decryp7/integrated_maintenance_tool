import { afterNextRender, ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, signal, untracked, viewChild } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { formatKeybinding } from '../extensions/keybinding';
import { ImtCommand } from '../extensions/tokens';
import { PaletteService } from './palette.service';

let nextId = 0;

/** Keyboard-driven command picker (⌘K / Ctrl+K, or the title bar search box). */
@Component({
  selector: 'imt-command-palette',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (palette.isOpen()) {
      <div class="backdrop" (mousedown)="close()"></div>
      <div class="box" role="dialog" aria-modal="true" aria-label="Command palette">
        <input
          #input
          class="input"
          role="combobox"
          aria-label="Type a command"
          aria-autocomplete="list"
          aria-expanded="true"
          [attr.aria-controls]="listId"
          [attr.aria-activedescendant]="activeId()"
          placeholder="Type a command…"
          autocomplete="off"
          spellcheck="false"
          [value]="query()"
          (input)="onInput($any($event.target).value)"
          (keydown)="onKey($event)"
        />
        <ul class="list" role="listbox" aria-label="Commands" [id]="listId">
          @for (c of results(); track c.id; let i = $index) {
            <li
              role="option"
              class="opt"
              [id]="optionId(i)"
              [class.active]="i === index()"
              [attr.aria-selected]="i === index()"
              (mousemove)="index.set(i)"
              (click)="run(c)"
            >
              @if (c.category) { <span class="cat">{{ c.category }}</span> }
              <span class="title">{{ c.title }}</span>
              @if (c.keybinding) { <kbd class="kbd">{{ hint(c) }}</kbd> }
            </li>
          } @empty {
            <li class="none" role="presentation">No matching commands</li>
          }
        </ul>
      </div>
    }
  `,
  styles: `
    .backdrop { position: fixed; inset: 0; z-index: 200; background: rgba(0, 0, 0, 0.35); }
    .box { position: fixed; z-index: 201; top: 72px; left: 50%; transform: translateX(-50%); width: min(560px, calc(100vw - 32px)); display: flex; flex-direction: column; gap: 6px; padding: 8px; border-radius: 10px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .input { height: 32px; }
    .list { list-style: none; margin: 0; padding: 0; max-height: 360px; overflow: auto; display: flex; flex-direction: column; }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 0 10px; border-radius: 6px; color: var(--text-2); cursor: pointer; }
    .opt.active { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .cat { color: var(--muted); font-size: 12px; }
    .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .kbd { font-family: var(--font-mono); font-size: 11px; color: var(--muted); }
    .none { padding: 10px; color: var(--muted); }
  `,
})
export class CommandPalette {
  protected readonly palette = inject(PaletteService);
  private readonly registry = inject(CommandRegistry);
  private readonly injector = inject(Injector);
  private readonly input = viewChild<ElementRef<HTMLInputElement>>('input');

  protected readonly listId = `imt-palette-list-${nextId++}`;
  protected readonly query = signal('');
  protected readonly index = signal(0);
  protected readonly results = computed(() => this.registry.search(this.query()));
  protected readonly activeId = computed(() => (this.results().length ? this.optionId(this.index()) : null));
  private returnFocus: HTMLElement | null = null;

  constructor() {
    effect(() => {
      if (this.palette.isOpen()) {
        untracked(() => {
          this.returnFocus = document.activeElement as HTMLElement | null;
          this.query.set('');
          this.index.set(0);
        });
        afterNextRender(() => this.input()?.nativeElement.focus(), { injector: this.injector });
      }
    });
  }

  protected optionId(i: number): string {
    return `${this.listId}-${i}`;
  }

  protected hint(c: ImtCommand): string {
    return formatKeybinding(c.keybinding ?? '');
  }

  protected onInput(value: string): void {
    this.query.set(value);
    this.index.set(0);
  }

  protected onKey(e: KeyboardEvent): void {
    const n = this.results().length;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (n) this.index.set((this.index() + 1) % n);
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (n) this.index.set((this.index() - 1 + n) % n);
        break;
      case 'Enter': {
        e.preventDefault();
        const c = this.results()[this.index()];
        if (c) this.run(c);
        break;
      }
      case 'Escape':
        e.preventDefault();
        this.close();
        break;
      case 'Tab':
        e.preventDefault();
        break;
    }
  }

  protected close(): void {
    this.palette.close();
    this.returnFocus?.focus?.();
    this.returnFocus = null;
  }

  protected run(c: ImtCommand): void {
    this.palette.close();
    this.returnFocus = null;
    void this.registry.run(c.id);
  }
}
