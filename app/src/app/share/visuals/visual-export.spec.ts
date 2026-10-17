import { TestBed } from '@angular/core/testing';
import { WorkspaceStore } from '../../core/workspace.store';
import { settle } from '../../../testing/helpers';
import { loadWorkspace } from '../../analysis/test-setup';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';
import { fileStem, SVG_RASTERIZER, svgSize, VISUAL_ECHARTS, VisualExportService } from './visual-export.service';
import { setupVisuals } from './visual-testing';
import { VisualCommands } from './visual.commands';
import { TimelineAnnotations } from '../../timeline/annotations.store';

describe('VisualExportService', () => {
  it('renders charts off screen as SVG and PNG, without animation or brushing, and cleans up', async () => {
    const { charts, service } = setupVisuals();
    const svg = await service.chartSvg({ animation: true, brush: { x: 1 }, toolbox: {}, series: [] });
    expect(svg).toContain('<text>chart</text>');
    expect(charts[0].opts).toEqual({ renderer: 'svg', ssr: true, width: 1000, height: 360 });
    expect(charts[0].option).toEqual({ animation: false, series: [] });
    expect(charts[0].disposed).toBe(true);
    const before = document.body.children.length;
    const png = await service.chartPng({ series: [] }, { width: 300, height: 200 });
    expect(png.startsWith('data:image/png')).toBe(true);
    expect(charts[1].opts.renderer).toBe('canvas');
    expect(charts[1].host?.style.width).toBe('300px');
    expect(charts[1].disposed).toBe(true);
    expect(document.body.children.length).toBe(before);
  });

  it('exports a chart through the save dialog and the engine', async () => {
    const { service, share, picks } = setupVisuals();
    const store = TestBed.inject(WorkspaceStore);
    expect(await service.exportChart({ series: [] }, 'CPU %', 'svg')).toBe('/out/cpu.svg');
    expect(await service.exportChart({ series: [] }, 'CPU %', 'png')).toBe('/out/cpu.png');
    expect(picks).toEqual(['cpu.svg', 'cpu.png']);
    expect(share.written.map((w) => w.kind)).toEqual(['svg', 'png']);
    expect(store.notice()).toBe('Saved cpu.png');
  });

  it('reports a cancelled dialog as nothing and failures as an error', async () => {
    const { service } = setupVisuals(() => null);
    expect(await service.exportChart({}, 'x', 'png')).toBeNull();
    const failing = setupVisualsFailing();
    expect(await failing.exportChart({}, 'x', 'png')).toBeNull();
    expect(TestBed.inject(WorkspaceStore).error()).toContain('Could not export the image: disk full');
  });

  it('serializes the loaded timeline with annotations and exports it as SVG and PNG', async () => {
    const { service, share } = setupVisuals();
    const store = await loadWorkspace();
    await settle(undefined, 6);
    expect(store.timeline()).not.toBeNull();
    TestBed.inject(TimelineAnnotations).items.set([{ annotator: 'a', owner: 'o', kind: 'k', from: store.timeline()!.from, to: store.timeline()!.to, instance: null, on: null, label: 'reboot', severity: 'info', evidenceQuery: store.query() }]);
    const svg = service.timelineSvg('light')!;
    expect(svg).toContain('<title>Timeline — ');
    expect(svg).toContain('reboot');
    store.setTheme('dark');
    expect(service.theme()).toBe('dark');
    store.setTheme('light');
    expect(service.theme()).toBe('light');
    expect(await service.exportTimeline('svg')).toBe('/out/timeline.svg');
    expect(await service.exportTimeline('png')).toBe('/out/timeline.png');
    expect(share.written.at(-1)?.kind).toBe('png');
  });

  it('has nothing to export without a timeline', async () => {
    const { service } = setupVisuals();
    expect(service.timelineSvg()).toBeNull();
    expect(await service.exportTimeline('svg')).toBeNull();
    expect(TestBed.inject(WorkspaceStore).error()).toContain('There is no timeline to export.');
    TestBed.inject(WorkspaceStore).setTheme('system');
    expect(['light', 'dark']).toContain(service.theme());
  });

  it('offers timeline commands only when there is a timeline', async () => {
    setupVisuals();
    const commands = TestBed.inject(VisualCommands).commands;
    const ctx = {} as never;
    expect(commands.map((c) => c.when?.(ctx))).toEqual([false, false]);
    await loadWorkspace();
    await settle(undefined, 6);
    expect(commands.map((c) => c.when?.(ctx))).toEqual([true, true]);
    await commands[0].run(ctx);
    await commands[1].run(ctx);
    expect((TestBed.inject(ShareBackend) as MockShareBackend).written.map((w) => w.path)).toEqual(['/out/timeline.png', '/out/timeline.svg']);
  });

  it('defaults to ECharts (loaded on first use) and a canvas rasterizer', async () => {
    TestBed.configureTestingModule({});
    expect(typeof (await TestBed.inject(VISUAL_ECHARTS)())).toBe('function');
    expect(typeof TestBed.inject(SVG_RASTERIZER)).toBe('function');
  });

  it('names files and reads SVG sizes', () => {
    expect(fileStem('  Events per interval by level! ')).toBe('events-per-interval-by-level');
    expect(fileStem('***')).toBe('chart');
    expect(svgSize('<svg width="640" height="200.5">')).toEqual({ width: 640, height: 200.5 });
    expect(svgSize('<svg viewBox="0 0 1 1">')).toEqual({ width: 1200, height: 400 });
    expect(svgSize('nothing', { width: 1, height: 2 })).toEqual({ width: 1, height: 2 });
  });
});

function setupVisualsFailing(): VisualExportService {
  TestBed.resetTestingModule();
  const { service, share } = setupVisuals();
  vi.spyOn(share, 'saveVisual').mockRejectedValue('disk full');
  return service;
}
