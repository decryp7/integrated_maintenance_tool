import { CmdError } from '../core/cmd-error';
import { TestBed } from '@angular/core/testing';
import { ANOMALY_ICONS, ANOMALY_LABELS, anomalyIcon, anomalyLabel, barWidth, direction, formatChange, levelCounts, shortId, templateQl } from './analysis-format';
import { qlQuote } from './ql-text';
import { AsyncResource, errorText } from './async-resource';
import { HISTORY_STORE, LocalHistoryStore, MAX_HISTORY, QueryHistory } from './history-store';
import { TimeCursor, TimeCursorLine, TimeCursorSource, timeToFrac, xToTime } from './time-cursor';
import { manualFrames } from '../../testing/helpers';
import { aroundTime, midpoint, previousWindow, span } from './windows';
import { Component, signal } from '@angular/core';
import { MemoryHistory } from './test-setup';

describe('analysis format', () => {
  it('labels and icons exist for every kind and fall back for unknown ones', () => {
    for (const k of Object.keys(ANOMALY_LABELS)) expect(ANOMALY_ICONS[k as keyof typeof ANOMALY_ICONS]).toBeTruthy();
    expect(anomalyLabel('crash')).toBe('Crash');
    expect(anomalyIcon('crash')).toBe('✖');
    expect(anomalyLabel('weird' as never)).toBe('weird');
    expect(anomalyIcon('weird' as never)).toBe('•');
  });

  it('formats change with sign, direction and arrows', () => {
    expect(formatChange(12.34)).toBe('+12.3%');
    expect(formatChange(-3)).toBe('−3%');
    expect(formatChange(0)).toBe('0%');
    expect(formatChange(null)).toBe('—');
    expect(formatChange(undefined)).toBe('—');
    expect(formatChange(Infinity)).toBe('—');
    expect(direction(5)).toBe('up');
    expect(direction(-5)).toBe('down');
    expect(direction(0.01)).toBe('flat');
    expect(direction(null)).toBe('flat');
    expect(direction(undefined)).toBe('flat');
  });

  it('computes bars, level counts, short ids', () => {
    expect(barWidth(0, 10)).toBe(0);
    expect(barWidth(5, 0)).toBe(0);
    expect(barWidth(0.001, 100)).toBe(2);
    expect(barWidth(50, 100)).toBe(50);
    expect(barWidth(500, 100)).toBe(100);
    expect(levelCounts([0, 2, 0, 0, 1])).toEqual([{ level: 'Debug', count: 2 }, { level: 'Error', count: 1 }]);
    expect(shortId('abcdef', 4)).toBe('abcd…');
    expect(shortId('abc', 4)).toBe('abc');
  });

  it('builds ql for templates', () => {
    expect(qlQuote('a"b\\c')).toBe('"a\\"b\\\\c"');
    expect(templateQl('Order {Id} placed', false)).toBe('@template = "Order {Id} placed"');
    expect(templateQl('Took <*> ms (x)', true)).toBe('@message ~ "Took .+ ms \\\\(x\\\\)"');
  });
});

describe('AsyncResource', () => {
  it('loads, reports errors, retries and lets the latest call win', async () => {
    const r = new AsyncResource<number>();
    expect(await r.retry()).toBeUndefined();
    await r.load(async () => 1);
    expect(r.data()).toBe(1);
    expect(r.loading()).toBe(false);
    await r.load(() => Promise.reject(new Error('boom')));
    expect(r.error()).toBe('boom');
    expect(r.retryable()).toBe(true);
    await r.load(() => Promise.reject(new CmdError({ code: 'invalid', message: 'bad', retryable: false, hint: 'Fix it.' })));
    expect([r.error(), r.retryable()]).toEqual(['bad Fix it.', false]);
    expect(r.data()).toBe(1);
    let fail = true;
    const flaky = async () => {
      if (fail) throw 'nope';
      return 7;
    };
    await r.load(flaky);
    expect(r.error()).toBe('nope');
    fail = false;
    await r.retry();
    expect(r.error()).toBeNull();
    expect(r.data()).toBe(7);

    let release!: (n: number) => void;
    const slow = r.load(() => new Promise<number>((res) => (release = res)));
    await r.load(async () => 9);
    release(1);
    await slow;
    expect(r.data()).toBe(9);

    let reject!: (e: unknown) => void;
    const slowFail = r.load(() => new Promise<number>((_, rej) => (reject = rej)));
    await r.load(async () => 10);
    reject('late');
    await slowFail;
    expect(r.error()).toBeNull();

    r.reset();
    expect(r.data()).toBeNull();
    expect(r.loading()).toBe(false);
    expect(errorText({ a: 1 })).toBe('{"a":1}');
  });
});

describe('query history', () => {
  afterEach(() => localStorage.clear());

  it('keeps recent unique queries, capped, and persists them', () => {
    const mem = new MemoryHistory(['old']);
    TestBed.configureTestingModule({ providers: [{ provide: HISTORY_STORE, useValue: mem }] });
    const h = TestBed.inject(QueryHistory);
    expect(h.entries()).toEqual(['old']);
    h.add('  ');
    h.add('a');
    h.add('old');
    expect(h.entries()).toEqual(['old', 'a']);
    expect(mem.saved).toEqual(['old', 'a']);
    for (let i = 0; i < MAX_HISTORY + 5; i++) h.add(`q${i}`);
    expect(h.entries()).toHaveLength(MAX_HISTORY);
    h.clear();
    expect(h.entries()).toEqual([]);
  });

  it('local store tolerates missing, corrupt and blocked storage', () => {
    const s = new LocalHistoryStore();
    expect(s.load()).toEqual([]);
    s.save(['x', 'y']);
    expect(s.load()).toEqual(['x', 'y']);
    localStorage.setItem('debrief.queryHistory.v1', '{"a":1}');
    expect(s.load()).toEqual([]);
    localStorage.setItem('debrief.queryHistory.v1', 'not json');
    expect(s.load()).toEqual([]);
    localStorage.setItem('debrief.queryHistory.v1', JSON.stringify(['a', 3, 'b']));
    expect(s.load()).toEqual(['a', 'b']);
    const spy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full');
    });
    expect(() => s.save(['z'])).not.toThrow();
    spy.mockRestore();
  });

  it('uses the local store by default', () => {
    expect(TestBed.inject(HISTORY_STORE)).toBeInstanceOf(LocalHistoryStore);
  });
});

describe('windows', () => {
  it('builds comparison windows', () => {
    expect(span({ from: 10, to: 30 })).toBe(20);
    expect(previousWindow({ from: 100, to: 150 })).toEqual({ from: 50, to: 100 });
    expect(previousWindow({ from: 100, to: 100 })).toEqual({ from: 99, to: 100 });
    expect(aroundTime(100, 10)).toEqual({ a: { from: 90, to: 100 }, b: { from: 100, to: 110 } });
    expect(aroundTime(100, 0).b.to).toBe(101);
    expect(midpoint({ from: 0, to: 11 })).toBe(6);
  });
});

describe('TimeCursor', () => {
  it('tracks hover and a normalised selection', () => {
    const c = TestBed.inject(TimeCursor);
    c.hover(5);
    expect(c.hovered()).toBe(5);
    c.hover(NaN);
    expect(c.hovered()).toBeNull();
    c.hover(null);
    c.select({ from: 9, to: 3 });
    expect(c.selection()).toEqual({ from: 3, to: 9 });
    c.select({ from: NaN, to: 3 });
    expect(c.selection()).toBeNull();
    c.select({ from: 1, to: 2 });
    c.select(null);
    expect(c.selection()).toBeNull();
    c.hover(4);
    c.select({ from: 1, to: 2 });
    c.clear();
    expect(c.hovered()).toBeNull();
    expect(c.selection()).toBeNull();
  });

  it('maps x to time inside the range', () => {
    expect(xToTime(50, 100, { from: 0, to: 1000 })).toBe(500);
    expect(xToTime(-5, 100, { from: 0, to: 1000 })).toBe(0);
    expect(xToTime(500, 100, { from: 0, to: 1000 })).toBe(1000);
    expect(xToTime(5, 0, { from: 7, to: 9 })).toBe(7);
  });

  it('the directive publishes the hovered time', () => {
    @Component({ imports: [TimeCursorSource], template: `<div id="plot" [debriefTimeCursor]="range()"></div>` })
    class Host {
      readonly range = signal<{ from: number; to: number } | null>({ from: 0, to: 1000 });
    }
    const fixture = TestBed.createComponent(Host);
    fixture.detectChanges();
    const plot = fixture.nativeElement.querySelector('#plot') as HTMLElement;
    plot.getBoundingClientRect = () => ({ left: 10, width: 200 }) as DOMRect;
    const cursor = TestBed.inject(TimeCursor);
    const frames = manualFrames();
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 110, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBe(500);
    plot.dispatchEvent(new MouseEvent('pointerleave'));
    expect(cursor.hovered()).toBeNull();
    fixture.componentInstance.range.set(null);
    fixture.detectChanges();
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 110, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBeNull();
    frames.restore();
  });

  it('the directive maps only the plot box given by its insets', () => {
    @Component({ imports: [TimeCursorSource], template: `<div id="plot" [debriefTimeCursor]="{ from: 0, to: 1000 }" [debriefTimeCursorLeft]="50" [debriefTimeCursorRight]="50"></div>` })
    class Host {}
    const fixture = TestBed.createComponent(Host);
    fixture.detectChanges();
    const plot = fixture.nativeElement.querySelector('#plot') as HTMLElement;
    plot.getBoundingClientRect = () => ({ left: 0, width: 300 }) as DOMRect;
    const cursor = TestBed.inject(TimeCursor);
    const frames = manualFrames();
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 150, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBe(500);
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 40, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBeNull();
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 251, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBeNull();
    frames.restore();
  });

  it('the directive takes an explicit plot width', () => {
    @Component({ imports: [TimeCursorSource], template: `<div id="plot" [debriefTimeCursor]="{ from: 0, to: 1000 }" [debriefTimeCursorLeft]="100" [debriefTimeCursorWidth]="400"></div>` })
    class Host {}
    const fixture = TestBed.createComponent(Host);
    fixture.detectChanges();
    const plot = fixture.nativeElement.querySelector('#plot') as HTMLElement;
    const cursor = TestBed.inject(TimeCursor);
    const frames = manualFrames();
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 200, bubbles: true }));
    frames.flush();
    expect(cursor.hovered()).toBe(250);
    frames.restore();
  });

  it('the directive publishes at most once per frame, and a leave drops a move still waiting', () => {
    @Component({ imports: [TimeCursorSource], template: `<div id="plot" [debriefTimeCursor]="{ from: 0, to: 1000 }"></div>` })
    class Host {}
    const fixture = TestBed.createComponent(Host);
    fixture.detectChanges();
    const plot = fixture.nativeElement.querySelector('#plot') as HTMLElement;
    let rects = 0;
    plot.getBoundingClientRect = () => (rects++, { left: 0, width: 1000 }) as DOMRect;
    const cursor = TestBed.inject(TimeCursor);
    const frames = manualFrames();
    const hover = vi.spyOn(cursor, 'hover');
    for (let x = 1; x <= 30; x++) plot.dispatchEvent(new MouseEvent('pointermove', { clientX: x, bubbles: true }));
    // Before: 30 publishes (and 30 layout reads), each re-rendering every view that draws the cursor.
    expect(hover).not.toHaveBeenCalled();
    frames.flush();
    expect(hover).toHaveBeenCalledTimes(1);
    expect(rects).toBe(1);
    expect(cursor.hovered()).toBe(30);
    plot.dispatchEvent(new MouseEvent('pointermove', { clientX: 500, bubbles: true }));
    plot.dispatchEvent(new MouseEvent('pointerleave'));
    frames.flush();
    expect(cursor.hovered()).toBeNull();
    frames.restore();
  });

  it('the cursor line follows the hovered time without re-rendering its view', async () => {
    let renders = 0;
    @Component({
      imports: [TimeCursorLine],
      template: `{{ count() }}<div class="line" [debriefTimeCursorLine]="{ from: 0, to: 1000 }" [debriefTimeCursorLineWidth]="width" [debriefTimeCursorLineLabel]="label"><span data-cursor-label></span></div>
        <div class="pct" [debriefTimeCursorLine]="{ from: 0, to: 1000 }" [debriefTimeCursorLineLeft]="10" [debriefTimeCursorLineRight]="30"></div>`,
    })
    class Host {
      width = 200;
      readonly label = (ts: number) => `at ${ts}`;
      count(): string {
        renders++;
        return '';
      }
    }
    const fixture = TestBed.createComponent(Host);
    await fixture.whenStable();
    const line = fixture.nativeElement.querySelector('.line') as HTMLElement;
    const pct = fixture.nativeElement.querySelector('.pct') as HTMLElement;
    expect(line.hidden).toBe(true);
    const cursor = TestBed.inject(TimeCursor);
    const before = renders;
    cursor.hover(250);
    await fixture.whenStable();
    expect(line.hidden).toBe(false);
    expect(line.style.left).toBe('50px');
    expect(line.textContent).toBe('at 250');
    expect(pct.style.left).toContain('10px');
    expect(pct.style.left).toContain('0.25');
    cursor.hover(2000);
    await fixture.whenStable();
    expect(line.hidden).toBe(true);
    expect(renders).toBe(before);
  });

  it('maps a time to its fraction of a range', () => {
    expect(timeToFrac(250, { from: 0, to: 1000 })).toBe(0.25);
    expect(timeToFrac(5, { from: 5, to: 5 })).toBe(0);
    expect(timeToFrac(-1, { from: 0, to: 1000 })).toBeNull();
    expect(timeToFrac(1001, { from: 0, to: 1000 })).toBeNull();
    expect(timeToFrac(null, { from: 0, to: 1000 })).toBeNull();
    expect(timeToFrac(5, null)).toBeNull();
  });
});
