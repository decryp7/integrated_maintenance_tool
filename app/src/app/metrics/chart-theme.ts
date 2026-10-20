import { inject, Injectable, linkedSignal } from '@angular/core';
import { ThemeService } from '../core/workspace/theme.service';
import { TokenSet } from '../theme/theme.types';

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

/** The chart tokens of a theme's token set (ECharts draws on a canvas and cannot read CSS variables). Pure. */
export function chartTokensOf(t: TokenSet): ChartTokens {
  const v = (name: string) => t[name] ?? '';
  return {
    text: v('text'),
    muted: v('muted'),
    faint: v('faint'),
    grid: v('grid-line'),
    border: v('border-3'),
    tooltipBg: v('tooltip-bg'),
    accentSoft: v('accent-soft'),
    accent: v('accent'),
    series: [v('s1'), v('s2'), v('s3'), v('s4')],
    other: v('s-other'),
    levels: {
      Verbose: v('lvl-verbose'),
      Debug: v('lvl-debug'),
      Information: v('hist-info'),
      Warning: v('lvl-warn'),
      Error: v('lvl-error'),
      Fatal: v('lvl-fatal'),
    },
    markers: {
      start: v('mk-start'),
      stop: v('mk-stop'),
      fatal: v('mk-fatal'),
      exception: v('mk-exception'),
      error: v('mk-error'),
      warning: v('mk-warning'),
    },
    fontSans: v('font-sans'),
    fontMono: v('font-mono'),
  };
}

/** The tokens charts draw with; they follow the theme, the colour scheme and the density in effect. */
@Injectable({ providedIn: 'root' })
export class ChartTheme {
  private readonly theme = inject(ThemeService);
  readonly tokens = linkedSignal<ChartTokens>(() => chartTokensOf(this.theme.tokens()));
}
