import { Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { ECharts } from 'echarts/core';
import { AnalysisBackend } from '../../core/analysis-backend';
import { ImtBackend } from '../../core/backend';
import { Platform } from '../../core/platform';
import { DockService } from '../../dock/dock.service';
import { FakeDock } from '../../../testing/helpers';
import { provideAnalysisTesting } from '../../analysis/test-setup';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';
import { OffscreenOpts, SVG_RASTERIZER, VISUAL_ECHARTS, VisualExportService } from './visual-export.service';

/** Test support for the visual exports and the report builder: demo workspace, mock share backend, fake ECharts, save dialog and rasterizer. */

interface FakeChart {
  opts: OffscreenOpts;
  host: HTMLElement | null;
  option: Record<string, unknown> | null;
  disposed: boolean;
}

/** Configures TestBed; `pick` answers the save dialog (null = cancelled). */
export function setupVisuals(pick: (name: string) => string | null = (name) => `/out/${name}`, extra: Provider[] = []) {
  const charts: FakeChart[] = [];
  const picks: string[] = [];
  TestBed.configureTestingModule({
    providers: [
      ...provideAnalysisTesting(),
      { provide: DockService, useValue: new FakeDock() },
      { provide: ShareBackend, useFactory: () => new MockShareBackend(TestBed.inject(ImtBackend), TestBed.inject(AnalysisBackend)) },
      { provide: Platform, useValue: Object.assign(new Platform(), { pickSavePath: async (_t: string, name: string) => (picks.push(name), pick(name)) }) },
      {
        provide: VISUAL_ECHARTS,
        useValue: async () => (host: HTMLElement | null, _theme: undefined, opts: OffscreenOpts) => {
          const c: FakeChart = { opts, host, option: null, disposed: false };
          charts.push(c);
          return {
            setOption: (o: Record<string, unknown>) => (c.option = o),
            renderToSVGString: () => '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>chart</text></svg>',
            getDataURL: () => 'data:image/png;base64,iVBORw0KGgo=',
            dispose: () => (c.disposed = true),
          } as unknown as ECharts;
        },
      },
      ...extra,
      { provide: SVG_RASTERIZER, useValue: async (svg: string, w: number, h: number, scale: number) => `data:image/png;base64,${btoa(`${w}x${h}@${scale}:${svg.length}`)}` },
    ],
  });
  return { charts, picks, service: TestBed.inject(VisualExportService), share: TestBed.inject(ShareBackend) as MockShareBackend };
}

