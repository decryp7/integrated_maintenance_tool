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
