import { EnvironmentProviders, Injectable, inject, provideEnvironmentInitializer } from '@angular/core';
import { MatIconRegistry } from '@angular/material/icon';
import { DomSanitizer } from '@angular/platform-browser';

/** Namespace of Debrief's own icon set in `MatIconRegistry` (`<mat-icon svgIcon="debrief:timeline">`). */
export const DEBRIEF_ICON_NAMESPACE = 'debrief';

/** Icon registered for names outside the set: an empty 24×24 canvas, so layout is unchanged. */
export const BLANK_ICON = 'blank';

/** Stroke icons drawn on a 24×24 grid: one path each, stroked with `currentColor`. */
export const ICON_PATHS: Readonly<Record<string, string>> = {
  files: 'M4 5h6l2 2h8v12H4z',
  search: 'M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l5 5',
  timeline: 'M3 7h10M7 12h14M3 17h8',
  metrics: 'M3 18l6-7l4 3l8-9',
  profile: 'M5 4h10l4 4v12H5zM9 11h6M9 15h6',
  settings: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1',
  plus: 'M12 5v14M5 12h14',
  minus: 'M5 12h14',
  close: 'M6 6l12 12M18 6L6 18',
  refresh: 'M19 12a7 7 0 1 1-2.1-5M19 4v4h-4',
  folder: 'M3 6h6l2 2h10v11H3z',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  chevronDown: 'M6 9l6 6l6-6',
  chevronRight: 'M9 6l6 6l-6 6',
  chevronLeft: 'M15 6l-6 6l6 6',
  globe: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18M3 12h18M12 3c-3 3.2-3 14.8 0 18M12 3c3 3.2 3 14.8 0 18',
  group: 'M8 12a3 3 0 1 0 0 .1M14 7h7M14 12h7M14 17h7',
  trash: 'M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13',
  copy: 'M9 9h11v11H9zM5 15V4h11',
  columns: 'M4 5h16v14H4zM10 5v14M16 5v14',
  sortAsc: 'M8 18V6M5 9l3-3l3 3M14 8h6M14 12h4M14 16h2',
  sortDesc: 'M8 6v12M5 15l3 3l3-3M14 8h2M14 12h4M14 16h6',
  sun: 'M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4',
  moon: 'M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z',
  check: 'M5 12.5l4.5 4.5L19 7',
  home: 'M4 11l8-7l8 7M6 9.5V20h12V9.5',
  alert: 'M12 4l9 16H3zM12 10v4M12 17v.5',
  zoomOut: 'M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l5 5M8 10.5h5',
  drag: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
  panel: 'M4 5h16v14H4zM15 5v14',
  chart: 'M4 20V4M4 20h16M8 16v-5M12 16V8M16 16v-3',
  puzzle: 'M5 9h4a2 2 0 1 1 4 0h4v4a2 2 0 1 1 0 4v3H5z',
  analysis: 'M4 4v16h16M7 15l3-4l3 2l4-6M15.5 15.5a2 2 0 1 0 0 .1',
  live: 'M12 11a1 1 0 1 0 0 2a1 1 0 1 0 0-2M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8',
  ai: 'M11 3l1.8 4.7L17.5 9.5l-4.7 1.8L11 16l-1.8-4.7L4.5 9.5l4.7-1.8zM18 15l.8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z',
  tools: 'M14.5 4a4.5 4.5 0 0 0-4.2 6.1L4 16.4V20h3.6l6.3-6.3A4.5 4.5 0 0 0 20 9.5l-2.8 2.8l-2.9-.8l-.8-2.9L16.3 5.8A4.5 4.5 0 0 0 14.5 4z',
  more: 'M5.5 12h.01M12 12h.01M18.5 12h.01',
  bell: 'M6 16V11a6 6 0 0 1 12 0v5l2 2H4zM10 20a2 2 0 0 0 4 0',
  jobs: 'M4 6h11M4 12h7M4 18h9M18 10v8M15 15l3 3l3-3',
  arrowUp: 'M12 19V5M6 11l6-6l6 6',
  arrowDown: 'M12 5v14M6 13l6 6l6-6',
  stop: 'M7 7h10v10H7z',
  research: 'M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l5 5M8 10.5h5M10.5 8v5',
  image: 'M4 5h16v14H4zM4 16l5-5l4 4l2-2l5 5M15.5 8.5a1 1 0 1 0 0 .1',
  note: 'M5 4h14v11l-5 5H5zM14 20v-5h5M8 9h8M8 12h5',
};

/** True when `name` is part of Debrief's icon set. */
export function isDebriefIcon(name: string): boolean {
  return Object.hasOwn(ICON_PATHS, name);
}

/**
 * SVG markup for one stroke path. Stroke width is not baked in: it is the inherited CSS
 * `stroke-width` of the host, so one registered literal serves every weight.
 */
export function iconSvg(path: string): string {
  const body = path ? `<path d="${path}"/>` : '';
  return (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    `stroke-linecap="round" stroke-linejoin="round">${body}</svg>`
  );
}

/**
 * Registers Debrief's icon set in the `debrief` namespace, once per registry. The markup comes only
 * from the constant {@link ICON_PATHS} table (never from user or plugin input), which is what
 * makes bypassing the sanitizer safe here. Literals mean no icon is ever fetched over the
 * network, as the CSP requires.
 */
@Injectable({ providedIn: 'root' })
export class DebriefIconSet {
  constructor() {
    const registry = inject(MatIconRegistry);
    const sanitizer = inject(DomSanitizer);
    const add = (name: string, path: string) =>
      registry.addSvgIconLiteralInNamespace(DEBRIEF_ICON_NAMESPACE, name, sanitizer.bypassSecurityTrustHtml(iconSvg(path)));
    for (const [name, path] of Object.entries(ICON_PATHS)) add(name, path);
    add(BLANK_ICON, '');
  }

  /** The registry name (`debrief:<name>`) to render `name`; unknown names map to the blank icon. */
  svgIcon(name: string): string {
    return `${DEBRIEF_ICON_NAMESPACE}:${isDebriefIcon(name) ? name : BLANK_ICON}`;
  }
}

/** Registers Debrief's icon set at startup so the first paint needs no lazy registration. */
export function provideDebriefIcons(): EnvironmentProviders {
  return provideEnvironmentInitializer(() => inject(DebriefIconSet));
}
