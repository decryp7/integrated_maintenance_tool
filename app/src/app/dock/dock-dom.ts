/**
 * A group never scrolls sideways. Focusing something wider than a group (a dialog opened from a narrow tool window)
 * would scroll the group's box (overflow: hidden still scrolls programmatically) and push its tab strip over the
 * neighbouring group's header. Vertical scrolling is left alone (a short query bar shows its status that way).
 */
export function keepGroupsUnscrolled(root: HTMLElement): { dispose(): void } {
  const onScroll = (e: Event) => {
    const t = e.target;
    if (t instanceof HTMLElement && t.classList.contains('dv-groupview') && t.scrollLeft !== 0) t.scrollLeft = 0;
  };
  root.addEventListener('scroll', onScroll, true);
  return { dispose: () => root.removeEventListener('scroll', onScroll, true) };
}

/**
 * Dockview marks floating groups `role="dialog"` with `aria-modal="false"` and a stacking
 * `aria-level` (0, 1, ...). axe rejects both on a dialog (aria-level is not allowed there and 0 is
 * invalid; aria-modal=false is the default), so they are removed as groups are floated or restored.
 */
export function observeFloatingAria(root: HTMLElement): { dispose(): void } {
  const fix = () =>
    root.querySelectorAll('.dv-resize-container[aria-modal], .dv-resize-container[aria-level]').forEach((el) => {
      el.removeAttribute('aria-modal');
      el.removeAttribute('aria-level');
    });
  const observer = new MutationObserver(fix);
  observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['aria-modal', 'aria-level'] });
  fix();
  return { dispose: () => observer.disconnect() };
}

/** The name and tooltip of a tab strip's overflow chip. */
export function overflowLabel(hidden: number): string {
  return hidden === 1
    ? $localize`:Tab strip overflow chip, one hidden tab@@dock.overflow.one:1 more tab`
    : $localize`:Tab strip overflow chip, several hidden tabs@@dock.overflow.many:${hidden}:count: more tabs`;
}

/**
 * dockview's overflow chip ("⌄ 2", the tabs a narrow strip hides) is a bare `div` that opens its list on click. It is
 * made a button here: named and titled "2 more tabs", focusable, and opened with Enter or Space (at the chip, since
 * dockview places the list at the click).
 */
export function labelTabOverflow(root: HTMLElement): { dispose(): void } {
  const label = () =>
    root.querySelectorAll<HTMLElement>('.dv-tabs-overflow-dropdown-root').forEach((chip) => {
      const hidden = Number.parseInt(chip.textContent?.trim() ?? '', 10);
      const name = overflowLabel(Number.isFinite(hidden) ? hidden : 0);
      if (chip.getAttribute('aria-label') !== name) {
        chip.setAttribute('aria-label', name);
        chip.title = name;
      }
      if (chip.getAttribute('role') === 'button') return;
      chip.setAttribute('role', 'button');
      chip.tabIndex = 0;
      chip.querySelectorAll('svg, span').forEach((el) => el.setAttribute('aria-hidden', 'true'));
    });
  const onKey = (e: KeyboardEvent) => {
    const chip = e.target instanceof HTMLElement && e.target.classList.contains('dv-tabs-overflow-dropdown-root') ? e.target : null;
    if (!chip || (e.key !== 'Enter' && e.key !== ' ')) return;
    e.preventDefault();
    const r = chip.getBoundingClientRect();
    // After this key event: the open list closes on Enter, and would see this one.
    setTimeout(() => chip.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: r.left, clientY: r.bottom })));
  };
  const observer = new MutationObserver(label);
  observer.observe(root, { childList: true, subtree: true, characterData: true });
  root.addEventListener('keydown', onKey);
  label();
  return {
    dispose: () => {
      observer.disconnect();
      root.removeEventListener('keydown', onKey);
    },
  };
}
