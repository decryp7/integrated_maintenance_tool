import { Size } from '../dock/float-cue';

/**
 * A PNG snapshot of a panel for the drag ghost, made inside the webview without anything the CSP forbids: the panel's
 * DOM is cloned (canvases become images of themselves), wrapped with the page's style sheets into an SVG
 * `foreignObject`, loaded as a `data:` image and drawn on a canvas. Where the webview refuses to read such a canvas
 * back (WebKit treats `foreignObject` images as tainted), or the panel is too large or slow to render, there is no
 * snapshot and the ghost shows the panel's icon and name only.
 */

/** Panels with more elements than this get no snapshot (cloning and rendering them would delay the drag). */
export const SNAPSHOT_MAX_NODES = 4000;
/** The largest snapshot sent to the shell (it accepts at most 2 MiB). */
export const SNAPSHOT_MAX_BYTES = 1_900_000;
/** How long rendering may take. */
export const SNAPSHOT_TIMEOUT_MS = 400;

const XHTML = 'http://www.w3.org/1999/xhtml';
/** Parts of a panel never worth drawing (and never safe to copy into another document). */
const DROPPED = 'script, iframe, object, embed, video, audio, .dv-drop-target-container, .dv-drop-target-dropzone, [data-imt-overlay]';

/** Draws an image URL on a canvas of `width`×`height` CSS px at `scale`: the PNG data URL, or null. */
export type Rasterize = (url: string, width: number, height: number, scale: number) => Promise<string | null>;

/** The scale that fits a panel into the ghost (never enlarged). */
export function snapshotScale(panel: Size, target: Size): number {
  if (!(panel.width > 0 && panel.height > 0)) return 0;
  return Math.min(1, target.width / panel.width, target.height / panel.height);
}

/** The SVG that renders `xhtml` (serialised XHTML) at `width`×`height`. */
export function snapshotSvg(xhtml: string, width: number, height: number): string {
  const w = Math.round(width);
  const h = Math.round(height);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><foreignObject x="0" y="0" width="${w}" height="${h}">${xhtml}</foreignObject></svg>`;
}

/** The SVG as an image URL. */
export function svgUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/**
 * A copy of the panel fit for another document: scripts, frames, media and dock overlays removed, canvases replaced
 * by pictures of themselves; null when the panel is too large to copy quickly.
 */
export function cloneForSnapshot(el: HTMLElement, maxNodes = SNAPSHOT_MAX_NODES): HTMLElement | null {
  if (el.getElementsByTagName('*').length > maxNodes) return null;
  const clone = el.cloneNode(true) as HTMLElement;
  const originals = el.querySelectorAll('canvas');
  clone.querySelectorAll('canvas').forEach((c, i) => {
    const source = originals[i];
    const img = el.ownerDocument.createElement('img');
    try {
      img.src = source?.toDataURL('image/png') ?? '';
    } catch {
      /* a tainted canvas: an empty box keeps the layout */
    }
    img.setAttribute('style', c.getAttribute('style') ?? '');
    img.width = source?.clientWidth || c.width;
    img.height = source?.clientHeight || c.height;
    c.replaceWith(img);
  });
  clone.querySelectorAll(DROPPED).forEach((n) => n.remove());
  // Event handler attributes would be inert in an image anyway; drop them so nothing executable is copied.
  for (const n of [clone, ...Array.from(clone.querySelectorAll('*'))]) {
    for (const a of Array.from(n.attributes)) if (a.name.toLowerCase().startsWith('on')) n.removeAttribute(a.name);
  }
  return clone;
}

/** The page's own style rules (cross-origin sheets, which IMT does not have, are skipped). */
export function documentCss(doc: Document): string {
  const parts: string[] = [];
  for (const sheet of Array.from(doc.styleSheets)) {
    try {
      for (const rule of Array.from(sheet.cssRules)) parts.push(rule.cssText);
    } catch {
      /* unreadable sheet */
    }
  }
  return parts.join('\n');
}

/** What the panel inherits from outside it: the theme's custom properties and the text style, as inline CSS. */
export function inheritedStyle(el: Element): string {
  const view = el.ownerDocument.defaultView;
  if (!view) return '';
  const cs = view.getComputedStyle(el);
  const decl: string[] = [];
  for (let i = 0; i < cs.length; i++) {
    const name = cs.item(i);
    if (name.startsWith('--')) decl.push(`${name}:${cs.getPropertyValue(name)}`);
  }
  for (const name of ['color', 'font-family', 'font-size', 'line-height', 'background-color', 'color-scheme']) {
    const v = cs.getPropertyValue(name);
    if (v) decl.push(`${name}:${v}`);
  }
  return decl.join(';').replace(/"/g, "'");
}

/** The serialised XHTML of the copy, wrapped with the styles it needs. */
export function snapshotDocument(el: HTMLElement, clone: HTMLElement, width: number, height: number): string {
  const doc = el.ownerDocument;
  const wrap = doc.createElementNS(XHTML, 'div') as HTMLElement;
  wrap.setAttribute('style', `${inheritedStyle(el)};width:${Math.round(width)}px;height:${Math.round(height)}px;overflow:hidden`);
  for (const a of ['data-theme', 'data-scheme', 'data-density']) {
    const v = doc.documentElement.getAttribute(a);
    if (v) wrap.setAttribute(a, v);
  }
  const style = doc.createElementNS(XHTML, 'style');
  style.textContent = documentCss(doc);
  wrap.append(style, clone);
  return new XMLSerializer().serializeToString(wrap);
}

function withTimeout<T>(p: Promise<T>, ms: number): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), ms);
    p.then(
      (v) => resolve(v),
      () => resolve(null),
    ).finally(() => clearTimeout(timer));
  });
}

/** The webview's canvas: null where it cannot draw or will not read back (a tainted canvas). */
export const canvasRaster: Rasterize = async (url, width, height, scale) => {
  const img = new Image();
  img.src = url;
  await img.decode();
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(width * scale));
  canvas.height = Math.max(1, Math.round(height * scale));
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.scale(scale, scale);
  ctx.drawImage(img, 0, 0, width, height);
  return canvas.toDataURL('image/png');
};

/**
 * A PNG data URL of the panel scaled to fit `target`, or null when no safe snapshot can be made (nothing to show,
 * too large, too slow, a webview that refuses). Never throws.
 */
export async function snapshotPanel(el: HTMLElement | null, target: Size, rasterize: Rasterize = canvasRaster, timeoutMs = SNAPSHOT_TIMEOUT_MS): Promise<string | null> {
  try {
    if (!el) return null;
    const box = el.getBoundingClientRect();
    const scale = snapshotScale(box, target);
    if (!scale) return null;
    const clone = cloneForSnapshot(el);
    if (!clone) return null;
    const url = svgUrl(snapshotSvg(snapshotDocument(el, clone, box.width, box.height), box.width, box.height));
    for (const s of [scale, scale / 2]) {
      const png = await withTimeout(rasterize(url, box.width, box.height, s), timeoutMs);
      if (!png || !png.startsWith('data:image/png;base64,')) return null;
      if (png.length <= SNAPSHOT_MAX_BYTES) return png;
    }
    return null;
  } catch {
    return null;
  }
}
