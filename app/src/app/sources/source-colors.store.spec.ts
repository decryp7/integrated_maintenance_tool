import { TestBed } from '@angular/core/testing';
import { all, provideTinyBackend, settle } from '../../testing/helpers';
import { at } from '../../testing/must';
import { Lane, RecordView } from '../core/api.types';
import { EventsGrid } from '../grid/events-grid';
import { Inspector } from '../inspector/inspector';
import { SettingsStore } from '../settings/settings.store';
import { Sidebar } from '../shell/sidebar';
import { TimelinePanel } from '../timeline/timeline-panel';
import { WorkspaceStore } from '../core/workspace.store';
import { SourceColors } from './source-colors.store';
import { SourceSwatch } from './source-swatch';
import { swatchAt } from './source-colors';

const row = (source: number, group: string): RecordView => ({
  id: 1,
  ts: 0, tsState: 'parsed',
  level: 'Information',
  message: 'm',
  template: null,
  exception: null,
  fields: {},
  source,
  sourceName: 's',
  line: 1, byteOffset: 0,
  group,
  markers: [],
});
const lane = (key: string, sources: number[]): Lane => ({ key, first: 0, last: 1, total: 1, matched: 1, density: [], markers: [], sources });

// Settings persist in localStorage: every test starts from the defaults.
beforeEach(() => localStorage.clear());

async function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  const store = TestBed.inject(WorkspaceStore);
  await store.refreshSummary();
  return { store, colors: TestBed.inject(SourceColors), settings: TestBed.inject(SettingsStore) };
}

describe('SourceColors', () => {
  it('colours lines by file id by default, with the file name as legend', async () => {
    const { colors } = await setup();
    expect(colors.mode()).toBe('source');
    expect(colors.forRecord(row(1, 'WEB02'))).toEqual({ swatch: swatchAt(1), label: 'File: app.log' });
    expect(colors.forSource(7)?.label).toBe('File: #7');
    expect(colors.forInstance('WEB02')).toBeNull();
    expect(colors.forLane(lane('WEB01', [0, 1, 2, 3, 4])).map((m) => m.swatch.slot)).toEqual([0, 1, 2]);
  });

  it('colours by instance in the order instances appear, and keeps them', async () => {
    const { colors, settings } = await setup();
    settings.setSourceColors('instance');
    expect(colors.forRecord(row(0, 'WEB02'))?.swatch.slot).toBe(0);
    expect(colors.forLane(lane('WEB01', [0]))).toEqual([{ swatch: swatchAt(1), label: 'Instance: WEB01' }]);
    expect(colors.forRecord(row(1, 'WEB02'))?.label).toBe('Instance: WEB02');
    expect(colors.forExplorer(at((await TestBed.inject(WorkspaceStore).summary())?.sources ?? [], 0))).toBeNull();
  });

  it('shows nothing when switched off', async () => {
    const { colors, settings } = await setup();
    settings.setSourceColors('off');
    expect(colors.enabled()).toBe(false);
    expect(colors.forRecord(row(0, 'x'))).toBeNull();
    expect(colors.forLane(lane('x', [0]))).toEqual([]);
  });
});

describe('source colour markers in the views', () => {
  it('the swatch draws its pattern and names itself', async () => {
    TestBed.configureTestingModule({});
    const fixture = TestBed.createComponent(SourceSwatch);
    fixture.componentRef.setInput('swatch', swatchAt(5));
    fixture.componentRef.setInput('label', 'File: x.log');
    await settle(fixture);
    const host = fixture.nativeElement as HTMLElement;
    expect(host.getAttribute('role')).toBe('img');
    expect(host.getAttribute('aria-label')).toBe('File: x.log');
    expect(host.getAttribute('title')).toBe('File: x.log');
    expect(host.querySelector('circle')?.getAttribute('stroke-dasharray')).toBe('4 2');
    fixture.componentRef.setInput('swatch', swatchAt(1));
    await settle(fixture);
    expect(host.querySelector('circle')?.getAttribute('fill')).toBe('var(--src-2)');
  });

  it('the grid draws a stripe per row with a legend, and none when off', async () => {
    const restore = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 400 });
    try {
      const { settings } = await setup();
      const fixture = TestBed.createComponent(EventsGrid);
      await settle(fixture);
      await settle(fixture);
      const stripes = all(fixture, '[data-testid=source-stripe]');
      expect(stripes.length).toBeGreaterThan(0);
      expect(stripes.map((s) => s.getAttribute('aria-label'))).toEqual(expect.arrayContaining(['File: app.clef', 'File: app.log']));
      expect(all(fixture, '.row.marked').length).toBe(stripes.length);
      settings.setSourceColors('off');
      await settle(fixture);
      expect(all(fixture, '[data-testid=source-stripe]')).toHaveLength(0);
      expect(all(fixture, '.row.marked')).toHaveLength(0);
    } finally {
      if (restore) Object.defineProperty(HTMLElement.prototype, 'clientHeight', restore);
    }
  });

  it('the Explorer, the inspector and the timeline lanes show matching dots', async () => {
    const { store } = await setup();
    const sidebar = TestBed.createComponent(Sidebar);
    await settle(sidebar);
    expect(all(sidebar, 'imt-source-swatch').map((s) => s.getAttribute('data-slot'))).toEqual(['0', '1']);

    const timeline = TestBed.createComponent(TimelinePanel);
    await settle(timeline);
    await settle(timeline);
    const dots = all(timeline, '.lane-label imt-source-swatch');
    expect(dots.length).toBeGreaterThan(0);
    expect(dots.every((d) => /^File: /.test(d.getAttribute('title') ?? ''))).toBe(true);

    const inspector = TestBed.createComponent(Inspector);
    store.select(row(1, 'WEB02'));
    await settle(inspector);
    expect(all(inspector, '.src-line imt-source-swatch').map((s) => s.getAttribute('data-slot'))).toEqual(['1']);
  });
});
