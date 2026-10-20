import { computed, DestroyRef, inject, InjectionToken, Signal, signal } from '@angular/core';

/**
 * Whether the dock panel a view is rendered in is visible (its tab is the active one of its group). A hidden panel is
 * not change-detected, so its effects do not run either. Provided by the dock's panel renderer; absent outside a
 * panel, which counts as always visible.
 */
export const PANEL_VISIBLE = new InjectionToken<Signal<boolean>>('PANEL_VISIBLE');

const ALWAYS: Signal<boolean> = signal(true).asReadonly();

/** The visibility of the panel the caller is rendered in (always visible outside a panel). Call in an injection context. */
export function panelVisible(): Signal<boolean> {
  return inject(PANEL_VISIBLE, { optional: true }) ?? ALWAYS;
}

/**
 * Tracks the views that use a root store, so the store only queries the engine while one of them is mounted and
 * visible. A store whose data feeds a view calls `active()` in its effects; each view calls `use()` once.
 */
export class ConsumerGate {
  private readonly consumers = signal<readonly { visible: Signal<boolean> }[]>([]);

  /** True while a registered view is mounted and visible. */
  readonly active = computed(() => this.consumers().some((c) => c.visible()));

  /** Registers the calling view for its lifetime. Call in an injection context (a component's constructor or field). */
  use(): void {
    this.add(panelVisible(), inject(DestroyRef));
  }

  /** Registers a consumer with an explicit visibility until `destroyRef` is destroyed. */
  add(visible: Signal<boolean>, destroyRef: Pick<DestroyRef, 'onDestroy'>): void {
    const entry = { visible };
    this.consumers.update((c) => [...c, entry]);
    destroyRef.onDestroy(() => this.consumers.update((c) => c.filter((e) => e !== entry)));
  }
}
