import { afterNextRender, ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, signal, untracked, viewChild } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { DockService } from '../dock/dock.service';
import { fuzzyFilter } from '../extensions/fuzzy';
import { ViewRegistry } from '../extensions/view-registry';
import { formatKeybinding } from '../extensions/keybinding';
import { ImtCommand } from '../extensions/tokens';
import { PaletteService } from './palette.service';
import { didYouMean } from './did-you-mean';

let nextId = 0;

/** Keyboard-driven command picker (⌘K / Ctrl+K, or the title bar search box). */
@Component({
  selector: 'imt-command-palette',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (palette.isOpen()) {
      <div class="backdrop" (mousedown)="close()"></div>
      <div class="box" role="dialog" aria-modal="true" [attr.aria-label]="dialogLabel()">
        <input
          #input
          class="input"
          role="combobox"
          [attr.aria-label]="inputLabel()"
          aria-autocomplete="list"
          aria-expanded="true"
          [attr.aria-controls]="listId"
          [attr.aria-activedescendant]="activeId()"
          [placeholder]="placeholder()"
          autocomplete="off"
          spellcheck="false"
          [value]="query()"
          (input)="onInput($any($event.target).value)"
          (keydown)="onKey($event)"
        />
        <ul class="list" role="listbox" [attr.aria-label]="listLabel()" [id]="listId">
          @if (suggesting()) {
            <li class="none" role="presentation" data-testid="palette-did-you-mean" i18n="@@palette.didYouMean">No commands match “{{ query() //i18n(ph="query") }}”. Did you mean:</li>
          }
          @for (c of options(); track c.id; let i = $index) {
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
            <li class="none" role="presentation">{{ emptyLabel() }}</li>
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
  private readonly viewRegistry = inject(ViewRegistry);
  private readonly injector = inject(Injector);
  private readonly input = viewChild<ElementRef<HTMLInputElement>>('input');

  protected readonly listId = `imt-palette-list-${nextId++}`;
  protected readonly query = signal('');
  protected readonly index = signal(0);
  /** Listing only the views (View ▸ Other windows…), each shown as an entry that opens it. */
  protected readonly views = computed(() => this.palette.mode() === 'views');
  private readonly viewEntries = computed<ImtCommand[]>(() =>
    this.viewRegistry.available().map((v) => ({ id: `view:${v.id}`, title: v.title, category: v.category === 'core' ? $localize`:@@palette.category.view:View` : (v.category ?? $localize`:@@palette.category.plugins:Plugins`), run: () => this.injector.get(DockService).show(v.id) })),
  );
  protected readonly results = computed(() =>
    this.views() ? fuzzyFilter(this.viewEntries(), this.query(), (c) => `${c.category}: ${c.title}`) : this.registry.search(this.query()),
  );
  /** Typo-tolerant suggestions, only when nothing matches. */
  protected readonly suggestions = computed(() =>
    this.results().length || this.views() || !this.query().trim() ? [] : didYouMean(this.registry.available(), this.query(), (c) => (c.category ? `${c.category} ${c.title}` : c.title)),
  );
  protected readonly suggesting = computed(() => this.suggestions().length > 0);
  protected readonly dialogLabel = computed(() => (this.views() ? $localize`:@@palette.dialog.windows:Other windows` : $localize`:@@palette.dialog.commands:Command palette`));
  protected readonly inputLabel = computed(() =>
    this.views() ? $localize`:@@palette.input.windowsLabel:Type a window name` : $localize`:@@palette.input.commandsLabel:Type a command`,
  );
  protected readonly placeholder = computed(() =>
    this.views() ? $localize`:@@palette.input.windowsPlaceholder:Type a window name…` : $localize`:@@palette.input.commandsPlaceholder:Type a command…`,
  );
  protected readonly listLabel = computed(() =>
    this.views()
      ? $localize`:@@palette.list.windows:Windows`
      : this.suggesting()
        ? $localize`:@@palette.list.suggested:Suggested commands`
        : $localize`:@@palette.list.commands:Commands`,
  );
  protected readonly emptyLabel = computed(() =>
    this.views() ? $localize`:@@palette.empty.windows:No matching windows` : $localize`:@@palette.empty.commands:No matching commands`,
  );
  /** What the list shows and the keyboard moves through: the matches, else the suggestions. */
  protected readonly options = computed(() => (this.results().length ? this.results() : this.suggestions()));
  protected readonly activeId = computed(() => (this.options().length ? this.optionId(this.index()) : null));
  private returnFocus: HTMLElement | null = null;

  constructor() {
    let wasOpen = false;
    effect(() => {
      const open = this.palette.isOpen();
      // Switching between commands and windows while open starts a new search too.
      this.palette.mode();
      if (open) {
        untracked(() => {
          if (!wasOpen) this.returnFocus = document.activeElement as HTMLElement | null;
          this.query.set('');
          this.index.set(0);
        });
        afterNextRender(() => this.input()?.nativeElement.focus(), { injector: this.injector });
      }
      wasOpen = open;
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
    const n = this.options().length;
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
        const c = this.options()[this.index()];
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
    if (this.views()) this.injector.get(DockService).show(c.id.slice('view:'.length));
    else void this.registry.run(c.id, 'palette');
  }
}
