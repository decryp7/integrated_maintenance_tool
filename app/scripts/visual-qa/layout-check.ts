import { Page } from '@playwright/test';

/** A layout defect found on the page: two controls/labels that overlap, or text cut off by its box. */
export interface LayoutProblem {
  kind: 'overlap' | 'clipped';
  a: string;
  b?: string;
}

/**
 * Finds visible controls and short labels that overlap each other, and text that overflows its box without an
 * ellipsis. Heuristic: it inspects leaf-ish elements (buttons, inputs, labels, badges, headings, short text spans) and
 * ignores elements inside scrollers' hidden areas, canvases and the timeline/grid internals that intentionally layer.
 */
export function layoutProblems(page: Page): Promise<LayoutProblem[]> {
  return page.evaluate(() => {
    const SELECTOR =
      'button, input, select, textarea, label, [role=switch], [role=checkbox], [role=tab], [role=menuitem], .badge, h1, h2, h3, h4, legend, span, a';
    // Timeline markers, annotation spans and lane contents layer on purpose; the checks cover the timeline's chrome.
    const IGNORE =
      'canvas, svg, .dv-sash, [aria-hidden=true], .cdk-visually-hidden, .sr-only, imt-timeline-panel .marker, imt-timeline-panel .lane, imt-timeline-panel .lane-label, imt-annotation-track';
    const describe = (el: Element) => {
      const text = (el.getAttribute('aria-label') || (el as HTMLElement).innerText || el.getAttribute('placeholder') || '').trim().slice(0, 40);
      const host = el.closest('[class*="imt-"], [ng-version]')?.tagName.toLowerCase() ?? '';
      let owner = el.parentElement;
      while (owner && !owner.tagName.includes('-')) owner = owner.parentElement;
      return `${owner?.tagName.toLowerCase() ?? host} ${el.tagName.toLowerCase()}${el.classList.length ? '.' + [...el.classList].join('.') : ''} "${text}"`;
    };
    /**
     * The part of the element's box that scrolling/overflow ancestors leave visible, and whether an ancestor that
     * hides overflow (not a scroller) cuts part of it off, i.e. the user cannot scroll to the rest.
     */
    const clipped = (el: Element): { r: DOMRect; cut: boolean } => {
      const own = el.getBoundingClientRect();
      let { left, top, right, bottom } = own;
      // Edges of overflow-hidden ancestors that cut the element, checked against the scrollable area afterwards.
      const cuts: { side: 'l' | 'r' | 't' | 'b'; at: number }[] = [];
      const hides = (o: string) => o === 'hidden' || o === 'clip';
      for (let p = el.parentElement; p; p = p.parentElement) {
        const s = getComputedStyle(p);
        if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
        const c = p.getBoundingClientRect();
        if (hides(s.overflowX) && own.left < c.right && own.right > c.left) {
          if (own.left < c.left - 1) cuts.push({ side: 'l', at: c.left });
          if (own.right > c.right + 1) cuts.push({ side: 'r', at: c.right });
        }
        if (hides(s.overflowY) && own.top < c.bottom && own.bottom > c.top) {
          if (own.top < c.top - 1) cuts.push({ side: 't', at: c.top });
          if (own.bottom > c.bottom + 1) cuts.push({ side: 'b', at: c.bottom });
        }
        left = Math.max(left, c.left);
        top = Math.max(top, c.top);
        right = Math.min(right, c.right);
        bottom = Math.min(bottom, c.bottom);
      }
      // A cut on the edge of the visible area is just scrolled out of view (the user can scroll to it); a cut inside
      // it hides content for good.
      const inside = (side: string, at: number) =>
        side === 'l' ? at > left + 1 : side === 'r' ? at < right - 1 : side === 't' ? at > top + 1 : at < bottom - 1;
      const edge = (side: string, at: number) =>
        Math.abs(at - (side === 'l' ? left : side === 'r' ? right : side === 't' ? top : bottom)) <= 1;
      const scrolledOut = (side: string) => {
        for (let p = el.parentElement; p; p = p.parentElement) {
          const s = getComputedStyle(p);
          const o = side === 'l' || side === 'r' ? s.overflowX : s.overflowY;
          if (o !== 'auto' && o !== 'scroll') continue;
          const c = p.getBoundingClientRect();
          const at = side === 'l' ? c.left : side === 'r' ? c.right : side === 't' ? c.top : c.bottom;
          if (edge(side, at)) return true;
        }
        return false;
      };
      const cut = cuts.some(({ side, at }) => inside(side, at) || (edge(side, at) && !scrolledOut(side)));
      return { r: new DOMRect(left, top, Math.max(0, right - left), Math.max(0, bottom - top)), cut };
    };
    /** Overlays (menus, dialogs, tooltips) legitimately cover the page: only elements in the same layer are compared. */
    const layer = (el: Element) => el.closest('.cdk-overlay-pane, [role=dialog], [role=menu], [role=tooltip]') ?? document.body;
    const visible = (el: Element, r: DOMRect) => {
      if (r.width < 2 || r.height < 2) return false;
      const s = getComputedStyle(el);
      if (s.visibility === 'hidden' || s.opacity === '0' || s.display === 'contents') return false;
      // Inside the viewport and not scrolled out of a clipping ancestor.
      if (r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth) return false;
      const cx = Math.min(Math.max(r.left + r.width / 2, 0), innerWidth - 1);
      const cy = Math.min(Math.max(r.top + r.height / 2, 0), innerHeight - 1);
      const top = document.elementFromPoint(cx, cy);
      return !!top && (el === top || el.contains(top) || top.contains(el));
    };
    const els = [...document.querySelectorAll(SELECTOR)].filter((el) => !el.closest(IGNORE));
    const boxes: { el: Element; r: DOMRect; cut: boolean; layer: Element }[] = [];
    for (const el of els) {
      // Only leaves among the candidates (a button's inner span is the same thing as the button).
      if (el.tagName === 'SPAN' && (el.children.length > 0 || !(el as HTMLElement).innerText.trim())) continue;
      const { r, cut } = clipped(el);
      if (visible(el, r)) boxes.push({ el, r, cut, layer: layer(el) });
    }
    const problems: { kind: 'overlap' | 'clipped'; a: string; b?: string }[] = [];
    const seen = new Set<string>();
    const push = (p: { kind: 'overlap' | 'clipped'; a: string; b?: string }) => {
      const key = `${p.kind}|${p.a}|${p.b ?? ''}`;
      if (!seen.has(key) && problems.length < 60) (seen.add(key), problems.push(p));
    };
    for (let i = 0; i < boxes.length; i++) {
      const { el: a, r: ra, cut, layer: la } = boxes[i];
      if (cut && !(a as HTMLElement).closest('.dv-tabs-container, [role=tablist]')) push({ kind: 'clipped', a: describe(a) });
      for (let j = i + 1; j < boxes.length; j++) {
        const { el: b, r: rb, layer: lb } = boxes[j];
        if (la !== lb || a.contains(b) || b.contains(a)) continue;
        const w = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
        const h = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
        if (w > 3 && h > 3) push({ kind: 'overlap', a: describe(a), b: describe(b) });
      }
      const s = getComputedStyle(a);
      const html = a as HTMLElement;
      const ellipsis = s.textOverflow === 'ellipsis';
      if (!ellipsis && html.innerText?.trim() && !['INPUT', 'TEXTAREA', 'SELECT'].includes(a.tagName)) {
        if (html.scrollWidth > html.clientWidth + 2 && s.overflowX !== 'visible' && s.overflowX !== 'auto' && s.overflowX !== 'scroll') {
          push({ kind: 'clipped', a: describe(a) });
        }
      }
    }
    return problems;
  });
}
