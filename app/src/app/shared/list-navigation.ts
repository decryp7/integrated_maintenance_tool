import { ActiveDescendantKeyManager, FocusableOption, FocusKeyManager, Highlightable, ListKeyManager } from '@angular/cdk/a11y';
import { computed, Injector, Signal } from '@angular/core';

/** An option known only by its index (the component renders the data; the key manager tracks the position). */
class IndexOption implements Highlightable, FocusableOption {
  constructor(
    readonly index: number,
    private readonly isDisabled: (index: number) => boolean,
    private readonly focusAt: (index: number) => void,
  ) {}

  get disabled(): boolean {
    return this.isDisabled(this.index);
  }

  setActiveStyles(): void {
    /* The template marks the active option from `active()`. */
  }

  setInactiveStyles(): void {
    /* Nothing: see setActiveStyles. */
  }

  focus(): void {
    this.focusAt(this.index);
  }
}

export interface ListNavigationOptions {
  /** ArrowDown on the last option goes to the first (and back). */
  wrap?: boolean;
  /** Home and End move to the first and last option (not for text inputs, where they move the caret). */
  homeAndEnd?: boolean;
  /** PageUp and PageDown move by this many options (0: not handled); a function for a size that changes (a viewport). */
  pageDelta?: number | (() => number);
  /** Options the keys skip. */
  disabled?: (index: number) => boolean;
  /** With no active option (a new list), the first counts as active, as in a combobox that highlights its first suggestion. */
  selectFirst?: boolean;
}

/**
 * Keyboard navigation over a list of `count` options with the CDK key managers, by index: the component keeps its
 * data and renders the active option from `active()`. Keys are matched by name (`KeyboardEvent.key`); the CDK's
 * own `onKeydown` reads the deprecated `keyCode`.
 */
abstract class ListNavigation<M extends ListKeyManager<IndexOption>> {
  protected readonly items: Signal<IndexOption[]>;
  readonly manager: M;
  /** Index of the active option; -1 when there is none. */
  readonly active: Signal<number>;
  private readonly pageDelta: () => number;
  private readonly homeAndEnd: boolean;
  private readonly selectFirst: boolean;

  protected constructor(count: Signal<number>, injector: Injector, options: ListNavigationOptions, create: (items: Signal<IndexOption[]>, injector: Injector) => M, focusAt: (index: number) => void) {
    const disabled = options.disabled ?? (() => false);
    this.items = computed(() => Array.from({ length: count() }, (_, i) => new IndexOption(i, disabled, focusAt)));
    this.manager = create(this.items, injector);
    this.manager.withWrap(options.wrap ?? false);
    const page = options.pageDelta ?? 0;
    this.pageDelta = typeof page === 'function' ? page : () => page;
    this.homeAndEnd = options.homeAndEnd ?? false;
    this.selectFirst = options.selectFirst ?? false;
    this.active = computed(() => {
      const i = this.manager.activeItemIndex ?? -1;
      const n = this.items().length;
      if (i >= 0 && i < n) return i;
      return this.selectFirst && n > 0 ? 0 : -1;
    });
  }

  /** Makes option `index` active (clamped; -1 or an empty list clears it). */
  setActive(index: number): void {
    const n = this.items().length;
    if (index < 0 || n === 0) {
      this.manager.updateActiveItem(-1);
      return;
    }
    this.manager.setActiveItem(Math.min(index, n - 1));
  }

  /** Makes option `index` active without any side effect (no focus): to follow a position kept elsewhere (a store). */
  sync(index: number): void {
    const n = this.items().length;
    this.manager.updateActiveItem(index < 0 || n === 0 ? -1 : Math.min(index, n - 1));
  }

  /**
   * Moves the active option for ArrowDown/ArrowUp (and Home/End, PageDown/PageUp when enabled). Returns true when the
   * key was handled (the caller prevents its default).
   */
  handle(e: KeyboardEvent): boolean {
    if (!this.items().length || e.altKey || e.ctrlKey || e.metaKey) return false;
    const m = this.manager;
    const current = m.activeItemIndex ?? -1;
    // Move from the option shown as active (the first, with selectFirst), not from "none".
    if (this.selectFirst && (current < 0 || current >= this.items().length) && /^(Arrow(Up|Down)|Page(Up|Down))$/.test(e.key)) m.updateActiveItem(0);
    switch (e.key) {
      case 'ArrowDown':
        m.setNextItemActive();
        return true;
      case 'ArrowUp':
        m.setPreviousItemActive();
        return true;
      case 'Home':
        if (!this.homeAndEnd) return false;
        m.setFirstItemActive();
        return true;
      case 'End':
        if (!this.homeAndEnd) return false;
        m.setLastItemActive();
        return true;
      case 'PageDown':
      case 'PageUp': {
        const delta = this.pageDelta();
        if (!delta) return false;
        const n = this.items().length;
        const from = Math.max(0, this.active());
        this.setActive(Math.max(0, Math.min(n - 1, from + (e.key === 'PageDown' ? delta : -delta))));
        return true;
      }
      default:
        return false;
    }
  }
}

/**
 * Focus stays where it is (a combobox input, a virtualised grid) and `aria-activedescendant` names the active option:
 * CDK `ActiveDescendantKeyManager`.
 */
export class ActiveDescendantNavigation extends ListNavigation<ActiveDescendantKeyManager<IndexOption>> {
  constructor(count: Signal<number>, injector: Injector, options: ListNavigationOptions = {}) {
    super(count, injector, options, (items, inj) => new ActiveDescendantKeyManager(items, inj), () => undefined);
  }
}

/**
 * Roving tab stop: the active option is the one focusable element (`tabindex=0`) and takes focus when it changes,
 * through `focusAt`: CDK `FocusKeyManager`.
 */
export class RovingFocusNavigation extends ListNavigation<FocusKeyManager<IndexOption>> {
  constructor(count: Signal<number>, injector: Injector, focusAt: (index: number) => void, options: ListNavigationOptions = {}) {
    super(count, injector, options, (items, inj) => new FocusKeyManager(items, inj), focusAt);
  }
}
