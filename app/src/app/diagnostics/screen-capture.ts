import { Injectable } from '@angular/core';
import { CapturedImage } from './snapshot.types';

export const MAX_WIDTH = 1600;
export const MAX_BYTES = 2 * 1024 * 1024;
const JPEG_QUALITY = 0.8;

/** The part of a canvas used for encoding (lets tests pass a fake). */
export interface EncodableCanvas {
  toDataURL(type?: string, quality?: number): string;
}

/** Scale that fits `width` into `MAX_WIDTH` (never enlarges). */
export function scaleFor(width: number): number {
  return width > MAX_WIDTH ? MAX_WIDTH / width : 1;
}

function parseDataUrl(url: string): CapturedImage | null {
  const m = /^data:(image\/(?:png|jpeg));base64,(.*)$/.exec(url);
  if (!m) return null;
  const base64 = m[2] ?? '';
  const padding = base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0;
  return { mime: m[1] as CapturedImage['mime'], base64, bytes: Math.floor((base64.length * 3) / 4) - padding };
}

/** PNG, or JPEG (quality 0.8) when the PNG is larger than `maxBytes`. */
export function encodeWithin(canvas: EncodableCanvas, maxBytes = MAX_BYTES): CapturedImage {
  const png = parseDataUrl(canvas.toDataURL('image/png'));
  if (png && png.bytes <= maxBytes) return png;
  const jpeg = parseDataUrl(canvas.toDataURL('image/jpeg', JPEG_QUALITY));
  if (!jpeg) throw new Error('The browser could not encode the screenshot.');
  return jpeg;
}

/** Prefix of Angular Material's theme tokens, which IMT maps onto its own tokens in `styles.scss`. */
const MATERIAL_TOKEN_PREFIX = '--mat-';

/**
 * The style properties copied onto each cloned node: everything the browser computes except Angular
 * Material's theme tokens. html-to-image copies every listed property to every node, and the ~165 tokens on
 * `:root` made a capture far slower; computed values already resolve `var()`, so the picture is unchanged.
 * IMT's own tokens stay, since inline SVG attributes such as `fill="var(--accent)"` read them.
 */
export function capturedStyleProperties(computed: Iterable<string>): string[] {
  return Array.from(computed).filter((name) => !name.startsWith(MATERIAL_TOKEN_PREFIX));
}

/** Renders the IMT window to an image. Replaceable in tests. */
@Injectable({ providedIn: 'root', useFactory: () => new DomScreenCapture() })
export abstract class ScreenCapture {
  abstract capture(): Promise<CapturedImage>;
}

/**
 * Draws the document into a canvas with html-to-image: no OS screen-recording permission, same on every
 * platform. Canvases (timeline, charts) are copied by html-to-image. Fonts are not embedded (the loader
 * would `fetch` them, which the app's `connect-src` forbids) and nothing leaves as a blob or remote URL: the
 * intermediate SVG is a `data:` URL, allowed by `img-src 'self' data:`.
 */
export class DomScreenCapture extends ScreenCapture {
  async capture(): Promise<CapturedImage> {
    const { toCanvas } = await import('html-to-image');
    const root = document.documentElement;
    const width = Math.max(1, root.clientWidth);
    const height = Math.max(1, root.clientHeight);
    const canvas = await toCanvas(document.body, {
      width,
      height,
      pixelRatio: scaleFor(width),
      skipFonts: true,
      includeStyleProperties: capturedStyleProperties(getComputedStyle(root)),
      cacheBust: false,
      backgroundColor: getComputedStyle(document.body).backgroundColor || '#ffffff',
      filter: (node) => !(node instanceof HTMLElement && node.hasAttribute('data-snapshot-ignore')),
    });
    return encodeWithin(canvas);
  }
}
