import { DestroyRef, inject, Injectable, signal } from '@angular/core';

/** Resolved CSS tokens for canvas rendering (ECharts cannot read var(--…)). */
export interface ChartTokens {
  text: string;
  muted: string;
  faint: string;
  grid: string;
  border: string;
  tooltipBg: string;
  accentSoft: string;
  accent: string;
  series: string[];
  other: string;
  levels: Record<string, string>;
  markers: Record<string, string>;
  fontSans: string;
  fontMono: string;
}

function read(): ChartTokens {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    text: v('--text'),
    muted: v('--muted'),
    faint: v('--faint'),
    grid: v('--grid-line'),
    border: v('--border-3'),
    tooltipBg: v('--tooltip-bg'),
    accentSoft: v('--accent-soft'),
    accent: v('--accent'),
    series: [v('--s1'), v('--s2'), v('--s3'), v('--s4')],
    other: v('--s-other'),
    levels: {
      Verbose: v('--lvl-verbose'),
      Debug: v('--lvl-debug'),
      Information: v('--hist-info'),
      Warning: v('--lvl-warn'),
      Error: v('--lvl-error'),
      Fatal: v('--lvl-fatal'),
    },
    markers: {
      start: v('--mk-start'),
      stop: v('--mk-stop'),
      fatal: v('--mk-fatal'),
      exception: v('--mk-exception'),
      error: v('--mk-error'),
      warning: v('--mk-warning'),
    },
    fontSans: v('--font-sans'),
    fontMono: v('--font-mono'),
  };
}

/** Re-reads tokens when the theme attribute or the OS colour scheme changes. */
@Injectable({ providedIn: 'root' })
export class ChartTheme {
  readonly tokens = signal<ChartTokens>(read());

  constructor() {
    const refresh = () => requestAnimationFrame(() => this.tokens.set(read()));
    const mo = new MutationObserver(refresh);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    const mq = window.matchMedia('(prefers-color-scheme: light)');
    mq.addEventListener('change', refresh);
    inject(DestroyRef).onDestroy(() => {
      mo.disconnect();
      mq.removeEventListener('change', refresh);
    });
  }
}
