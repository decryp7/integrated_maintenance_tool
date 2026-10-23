import { afterNextRender, ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, signal, untracked, viewChild } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { DockService } from '../dock/dock.service';
import { PaletteEntry, plainEntry, rankCommands, TitlePart, titleParts } from './palette-match';
import { COMMON_COMMANDS, PaletteRecent } from './palette-recent';
import { ViewRegistry } from '../extensions/view-registry';
import { formatKeybinding } from '../extensions/keybinding';
import { DebriefCommand, VIEW_OPEN_PREFIX } from '../extensions/tokens';
import { PaletteService } from './palette.service';
import { didYouMean } from './did-you-mean';
import { ActiveDescendantNavigation } from '../shared/list-navigation';

let nextId = 0;

/** Keyboard-driven command picker (⌘K / Ctrl+K, or the title bar search box). */
@Component({
  selector: 'debrief-command-palette',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (palette.isOpen()) {
      <div class="backdrop" data-debrief-overlay (mousedown)="close()"></div>
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
          @for (e of options(); track e.command.id; let i = $index) {
            <li
              role="option"
              class="opt"
              [id]="optionId(i)"
              [class.active]="i === index()"
              [attr.aria-selected]="i === index()"
              (mousemove)="nav.setActive(i)"
              (click)="run(e.command)"
            >
              @if (e.showCategory) { <span class="cat">{{ e.command.category }}</span> }
              <span class="title">@for (p of parts(e); track $index) {@if (p.mark) {<mark>{{ p.text }}</mark>} @else {{{ p.text }}}}</span>
              @if (e.command.keybinding) { <kbd class="kbd">{{ hint(e.command) }}</kbd> }
            </li>
          } @empty {
            <li class="none" role="presentation">{{ emptyLabel() }}</li>
          }
        </ul>
      </div>
    }
  `,
  styles: `
    .backdrop { position: fixed; inset: 0; z-index: 200; background: var(--scrim); }
    .box { position: fixed; z-index: 201; top: 72px; left: 50%; transform: translateX(-50%); width: min(560px, calc(100vw - 32px)); display: flex; flex-direction: column; gap: 6px; padding: 8px; border-radius: 10px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .input { height: 32px; }
    .list { list-style: none; margin: 0; padding: 0; max-height: 360px; overflow: auto; display: flex; flex-direction: column; }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 0 10px; border-radius: 6px; color: var(--text-2); cursor: pointer; }
    .opt.active { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .cat { color: var(--muted); font-size: var(--fs-sm); }
    .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .title mark { background: none; color: var(--accent-text); font-weight: 600; }
    .kbd { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--muted); }
    .none { padding: 10px; color: var(--muted); }
  `,
})
export class CommandPalette {
  protected readonly palette = inject(PaletteService);
  private readonly registry = inject(CommandRegistry);
  private readonly viewRegistry = inject(ViewRegistry);
  private readonly recent = inject(PaletteRecent);
  private readonly injector = inject(Injector);
  private readonly input = viewChild<ElementRef<HTMLInputElement>>('input');

  protected readonly listId = `debrief-palette-list-${nextId++}`;
  protected readonly query = signal('');
  /** Listing only the views (View ▸ Other windows…), each shown as an entry that opens it. */
  protected readonly views = computed(() => this.palette.mode() === 'views');
  private readonly viewEntries = computed<DebriefCommand[]>(() =>
    this.viewRegistry.available().map((v) => ({ id: `view:${v.id}`, title: v.title, category: v.category === 'core' ? $localize`:@@palette.category.view:View` : (v.category ?? $localize`:@@palette.category.plugins:Plugins`), run: () => this.injector.get(DockService).show(v.id) })),
  );
  /**
   * The palette's list: one entry per view (the generated "Open <view>" is left out when the view has its own command,
   * such as "Show anomalies"), ranked by {@link rankCommands}; empty, it starts with the recently run and the common
   * commands.
   */
  protected readonly results = computed<PaletteEntry[]>(() => {
    if (this.views()) return rankCommands(this.viewEntries(), this.query());
    const commands = this.registry.available();
    const own = new Set(commands.filter((c) => c.view && !c.id.startsWith(VIEW_OPEN_PREFIX)).map((c) => c.view));
    const listed = commands.filter((c) => !c.id.startsWith(VIEW_OPEN_PREFIX) || !own.has(c.view));
    return rankCommands(listed, this.query(), this.recent.ids(), COMMON_COMMANDS);
  });
  /** Typo-tolerant suggestions, only when nothing matches. */
  protected readonly suggestions = computed<PaletteEntry[]>(() =>
    this.results().length || this.views() || !this.query().trim()
      ? []
      : didYouMean(this.registry.available(), this.query(), (c) => (c.category ? `${c.category} ${c.title}` : c.title)).map(plainEntry),
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
  /** The highlighted option (CDK ActiveDescendantKeyManager; focus stays in the input). */
  protected readonly nav = new ActiveDescendantNavigation(computed(() => this.options().length), this.injector, { wrap: true, selectFirst: true });
  protected readonly index = this.nav.active;
  protected readonly activeId = computed(() => (this.options().length ? this.optionId(this.index()) : null));
  private returnFocus: HTMLElement | null = null;

  constructor() {
    let wasOpen = false;
    // effect-state: resets the search and focus each time the palette opens (an event, not derived state)
    effect(() => {
      const open = this.palette.isOpen();
      // Switching between commands and windows while open starts a new search too.
      this.palette.mode();
      if (open) {
        untracked(() => {
          if (!wasOpen) this.returnFocus = document.activeElement as HTMLElement | null;
          this.query.set('');
          this.nav.setActive(0);
        });
        afterNextRender(() => this.input()?.nativeElement.focus(), { injector: this.injector });
      }
      wasOpen = open;
    });
  }

  protected optionId(i: number): string {
    return `${this.listId}-${i}`;
  }

  protected hint(c: DebriefCommand): string {
    return formatKeybinding(c.keybinding ?? '');
  }

  protected onInput(value: string): void {
    this.query.set(value);
    this.nav.setActive(0);
  }

  protected onKey(e: KeyboardEvent): void {
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        e.preventDefault();
        this.nav.handle(e);
        break;
      case 'Enter': {
        e.preventDefault();
        const c = this.options()[this.index()];
        if (c) this.run(c.command);
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

  protected run(c: DebriefCommand): void {
    this.palette.close();
    this.returnFocus = null;
    if (this.views()) this.injector.get(DockService).show(c.id.slice('view:'.length));
    else {
      this.recent.add(c.id);
      void this.registry.run(c.id, 'palette');
    }
  }

  protected parts(e: PaletteEntry): TitlePart[] {
    return titleParts(e.title, e.marks);
  }
}
