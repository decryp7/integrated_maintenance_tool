import { inject, Injectable, InjectionToken } from '@angular/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { Platform } from '../../core/platform';
import { TimelineAnnotations } from '../../timeline/annotations.store';
import { errorMessage } from '../../tools/tool-errors';
import { ShareBackend } from '../share-backend';
import { SVG_PALETTES, SvgTheme, timelineToSvg } from './timeline-svg';
import { OverwriteGuard } from '../../ui/overwrite-guard';

export type VisualFormat = 'png' | 'svg';

/** Options of an off-screen chart instance. */
export interface OffscreenOpts {
  renderer: 'svg' | 'canvas';
  ssr?: boolean;
  width: number;
  height: number;
}

/** Creates an off-screen ECharts instance. */
export type OffscreenInit = (host: HTMLElement | null, theme: undefined, opts: OffscreenOpts) => ECharts;

/**
 * Loads ECharts' `init` for off-screen exports. ECharts is loaded on first use, so it stays out of the
 * initial bundle (the commands that export are registered at startup); tests provide a fake.
 */
export const VISUAL_ECHARTS = new InjectionToken<() => Promise<OffscreenInit>>('VISUAL_ECHARTS', {
  providedIn: 'root',
  factory: () => () => import('./offscreen-echarts').then((m) => m.offscreenInit),
});

/** Turns SVG markup into a PNG data URL of the given size (a canvas in the browser; tests provide a fake). */
export const SVG_RASTERIZER = new InjectionToken<(svg: string, width: number, height: number, scale: number) => Promise<string>>('SVG_RASTERIZER', {
  providedIn: 'root',
  factory: () => rasterize,
});

/* v8 ignore start: needs a real image decoder and canvas (exercised in end-to-end tests) */
function rasterize(svg: string, width: number, height: number, scale: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement('canvas');
      canvas.width = Math.round(width * scale);
      canvas.height = Math.round(height * scale);
      const ctx = canvas.getContext('2d');
      if (!ctx) return reject(new Error($localize`:@@share.visualExport.noCanvas:This browser cannot draw images.`));
      ctx.scale(scale, scale);
      ctx.drawImage(img, 0, 0, width, height);
      resolve(canvas.toDataURL('image/png'));
    };
    img.onerror = () => reject(new Error($localize`:@@share.visualExport.drawFailed:The image could not be drawn.`));
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  });
}
/* v8 ignore stop */

export const CHART_SIZE = { width: 1000, height: 360 } as const;

/** File name for a title: letters, digits and dashes. */
export function fileStem(title: string): string {
  return title.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'chart';
}

/** Width and height of an SVG document from its root attributes (fallback when absent). */
export function svgSize(svg: string, fallback = { width: 1200, height: 400 }): { width: number; height: number } {
  const root = /<svg\b[^>]*>/.exec(svg)?.[0] ?? '';
  const num = (name: string) => Number(new RegExp(`\\s${name}="([\\d.]+)"`).exec(root)?.[1]);
  const width = num('width');
  const height = num('height');
  return { width: width > 0 ? width : fallback.width, height: height > 0 ? height : fallback.height };
}

/**
 * Exports charts and the timeline as PNG or SVG, theme-aware. Charts are re-rendered off screen from
 * their ECharts option (SVG with ECharts' SVG renderer, PNG with its canvas renderer); the timeline is
 * serialized from its model. Files are written by the engine (`save_visual`), which checks the path and
 * sanitises the SVG.
 */
@Injectable({ providedIn: 'root' })
export class VisualExportService {
  private readonly store = inject(WorkspaceStore);
  private readonly annotations = inject(TimelineAnnotations);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly backend = inject(ShareBackend);
  private readonly loadECharts = inject(VISUAL_ECHARTS);
  private readonly rasterizer = inject(SVG_RASTERIZER);

  /** The theme the user sees (resolving "system"). */
  theme(): SvgTheme {
    const t = this.store.theme();
    if (t === 'light' || t === 'dark') return t;
    return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }

  /** The option without animation or interaction (brushing, toolbox): a still image. */
  static staticOption(option: EChartsCoreOption): EChartsCoreOption {
    const rest: Record<string, unknown> = { ...option, animation: false };
    delete rest['brush'];
    delete rest['toolbox'];
    return rest as EChartsCoreOption;
  }

  /** The chart as SVG markup (server-side rendering, nothing attached to the page). */
  async chartSvg(option: EChartsCoreOption, size: { width: number; height: number } = CHART_SIZE): Promise<string> {
    const chart = (await this.loadECharts())(null, undefined, { renderer: 'svg', ssr: true, ...size });
    try {
      chart.setOption(VisualExportService.staticOption(option));
      return chart.renderToSVGString();
    } finally {
      chart.dispose();
    }
  }

  /** The chart as a PNG data URL (twice the size, for sharp images), on the theme's background. */
  async chartPng(option: EChartsCoreOption, size: { width: number; height: number } = CHART_SIZE): Promise<string> {
    const init = await this.loadECharts();
    const host = document.createElement('div');
    host.style.cssText = `position:fixed;left:-10000px;top:0;width:${size.width}px;height:${size.height}px;`;
    document.body.appendChild(host);
    const chart = init(host, undefined, { renderer: 'canvas', ...size });
    try {
      chart.setOption(VisualExportService.staticOption(option));
      return chart.getDataURL({ type: 'png', pixelRatio: 2, backgroundColor: SVG_PALETTES[this.theme()].background });
    } finally {
      chart.dispose();
      host.remove();
    }
  }

  /** The current timeline as SVG, or null when there is none. */
  timelineSvg(theme: SvgTheme = this.theme()): string | null {
    const timeline = this.store.timeline();
    if (!timeline) return null;
    const kinds = this.store.markerKindMap();
    const markerColors = Object.fromEntries([...kinds.values()].map((k) => [k.kind, k.color]));
    const annotations = this.annotations
      .items()
      .filter((a) => this.annotations.isShown(a.kind))
      .map((a) => ({ from: a.from, to: a.to, label: a.label, color: kinds.get(a.kind)?.color ?? null }));
    const profile = this.store.summary()?.profileName ?? 'IMT';
    const title = $localize`:@@share.visualExport.timelineTitle:Timeline — ${profile}:profile:`;
    return timelineToSvg({ timeline, title, markerColors, annotations, theme, tz: this.store.timezone() });
  }

  /** Asks where to save, then writes the image. Returns the saved path, or null when cancelled or failed (reported). */
  private async save(kind: VisualFormat, title: string, data: () => Promise<string> | string): Promise<string | null> {
    try {
      const path = await this.platform.pickSavePath(
        kind === 'png' ? $localize`:@@share.visualExport.pickPng:Export PNG image` : $localize`:@@share.visualExport.pickSvg:Export SVG image`,
        `${fileStem(title)}.${kind}`,
        [{ name: kind === 'png' ? $localize`:@@share.visualExport.filterPng:PNG image` : $localize`:@@share.visualExport.filterSvg:SVG image`, extensions: [kind] }],
      );
      if (!path) return null;
      const image = await data();
      const saved = await this.overwrite.write(path, (overwrite) => this.backend.saveVisual({ path, kind, data: image, overwrite }));
      if (!saved) return null;
      this.store.setNotice($localize`:@@share.visualExport.saved:Saved ${saved.path.split(/[\\/]/).pop()}:file:`);
      return saved.path;
    } catch (e) {
      this.store.setError($localize`:@@share.visualExport.exportFailed:Could not export the image: ${errorMessage(e)}:error:`);
      return null;
    }
  }

  exportChart(option: EChartsCoreOption, title: string, kind: VisualFormat): Promise<string | null> {
    return this.save(kind, title, () => (kind === 'svg' ? this.chartSvg(option) : this.chartPng(option)));
  }

  exportTimeline(kind: VisualFormat): Promise<string | null> {
    return this.save(kind, 'timeline', async () => {
      const svg = this.timelineSvg();
      if (!svg) throw new Error($localize`:@@share.visualExport.noTimeline:There is no timeline to export.`);
      if (kind === 'svg') return svg;
      const { width, height } = svgSize(svg);
      return this.rasterizer(svg, width, height, 2);
    });
  }
}
