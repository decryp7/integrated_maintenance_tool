import { TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../../testing/helpers';
import { Logger } from '../logger';
import { ActivityLog } from './activity-log';
import { fingerprint, isSecretKey, safeParam, safeParams, safePath, safeText } from './privacy';
import { activity, ActivityRuntime, Clock, errorCode, isCancellation, LINGER_MS } from './runtime';
import { isUserError } from './severity';
import { steps } from './steps';
import { classifyKey, classifyTarget, TriggerTracker } from './trigger';
import { Breadcrumb } from './types';
import { withOpHeader } from './ipc';

import { at } from '../../../testing/must';
/** A clock the test moves by hand. */
export class FakeClock implements Clock {
  t = 1_000_000;
  private timers: { at: number; fn: () => void; id: number }[] = [];
  private next = 1;
  now = () => this.t;
  setTimeout = (fn: () => void, ms: number) => {
    const id = this.next++;
    this.timers.push({ at: this.t + ms, fn, id });
    return id;
  };
  clearTimeout = (h: unknown) => {
    this.timers = this.timers.filter((t) => t.id !== h);
  };
  advance(ms: number): void {
    this.t += ms;
    const due = this.timers.filter((t) => t.at <= this.t);
    this.timers = this.timers.filter((t) => t.at > this.t);
    due.forEach((t) => t.fn());
  }
}

function runtime(): { rt: ActivityRuntime; clock: FakeClock; out: { level: string; message: string; props: Record<string, string> }[] } {
  const clock = new FakeClock();
  const rt = new ActivityRuntime(clock);
  rt.window = 'main';
  rt.view = () => 'Events';
  const out: { level: string; message: string; props: Record<string, string> }[] = [];
  rt.setSink((level, message, props) => out.push({ level, message, props }));
  return { rt, clock, out };
}

describe('activity privacy', () => {
  it('fingerprints like the engine (FNV-1a 32)', () => {
    // Same vectors as app/src-tauri/src/activity/privacy.rs.
    expect(fingerprint('')).toBe('811c9dc5');
    expect(fingerprint('a')).toBe('e40c292c');
    expect(fingerprint('é')).toHaveLength(8);
    expect(fingerprint('level:error')).not.toBe(fingerprint('level:warn'));
  });

  it('keeps query text out in normal mode', () => {
    expect(safeText('Query', 'user:alice AND failed', 'normal')).toEqual({ QueryHash: fingerprint('user:alice AND failed'), QueryLength: 21 });
    expect(safeText('Query', 'user:alice', 'detailed')['Query']).toBe('user:alice');
    expect(safeText('Query', 'password=hunter2', 'detailed')['Query']).toBe('***');
  });

  it('hashes folders in normal mode', () => {
    const p = safePath('/Users/alice/secret-project/app.log', 'normal');
    expect(p).toMatch(/^<dir#[0-9a-f]{8}>\/app\.log$/);
    expect(p).not.toContain('alice');
    expect(safePath('C:\\Users\\bob\\x.log', 'normal')).toMatch(/^<dir#[0-9a-f]{8}>\/x\.log$/);
    expect(safePath('/Users/alice/app.log', 'detailed')).toBe('/Users/alice/app.log');
    expect(safePath('app.log', 'normal')).toBe('app.log');
    expect(safePath('/logs/', 'normal')).toBe('/logs');
  });

  it('masks secrets by name and content, redacts paths, caps and drops empties', () => {
    expect(isSecretKey('serviceApiKey')).toBe(true);
    expect([isSecretKey('Keys'), isSecretKey('Keybinding'), isSecretKey('api_key_id'), isSecretKey('Authorization')]).toEqual([false, false, true, true]);
    expect(safeParam('Token', 'abc')).toBe('***');
    expect(safeParam('Note', 'Bearer abc.def')).toBe('***');
    expect(safeParam('Note', 'key sk-abcdefghijkl here')).toBe('key *** here');
    expect(safeParam('Where', 'cannot open /etc/secret/file.log')).toBe('cannot open <path>');
    expect(safeParam('Where', '/etc/secret/file.log', 'detailed')).toBe('/etc/secret/file.log');
    expect(safeParam('Long', 'x'.repeat(300))).toHaveLength(201);
    expect(safeParams({ A: 1, B: true, C: null, D: undefined })).toEqual({ A: '1', B: 'true' });
    const many = Object.fromEntries(Array.from({ length: 30 }, (_, i) => [`K${i}`, i]));
    expect(Object.keys(safeParams(many))).toHaveLength(16);
  });

  it('tells user mistakes from unexpected errors', () => {
    expect(isUserError('invalid regex')).toBe(true);
    expect(isUserError(new Error('No such file or directory'))).toBe(true);
    expect(isUserError(new DOMException('x', 'AbortError'))).toBe(true);
    // Typed engine errors are decided by their code, whatever the wording (regression: a cleared job's "this job is
    // no longer listed" was reported as an app failure with a screenshot).
    expect(isUserError({ code: 'notFound', message: 'this job is no longer listed' })).toBe(true);
    expect(isUserError({ code: 'cancelled', message: 'stopped' })).toBe(true);
    expect(isUserError({ code: 'internal', message: 'invalid state' })).toBe(false);
    expect(isUserError({ code: 'engine', message: 'not found' })).toBe(false);
    // A research page that failed in the browser is the web's doing, not a Debrief bug (regression: reported as
    // "the model engine failed: the browser: ...").
    expect(isUserError({ code: 'browser', message: 'The browser could not finish: the page did not answer in time' })).toBe(true);
    expect(isUserError(new Error('index out of bounds'))).toBe(false);
    expect(isUserError({ circular: true })).toBe(false);
  });
});

describe('ActivityRuntime', () => {
  it('tracks success with duration, view, window and safe params', async () => {
    const { rt, clock, out } = runtime();
    const result = await rt.track('export.csv', 'menu', { Title: 'Export CSV', Path: '/home/a/x.csv' }, async () => {
      expect(rt.currentOperationId()).toBeTruthy();
      clock.advance(42);
      return 7;
    });
    expect(result).toBe(7);
    expect(out).toHaveLength(1);
    const { level, message, props } = at(out, 0);
    expect(level).toBe('info');
    expect(message).toBe('User export.csv via menu');
    expect(props).toMatchObject({
      SourceContext: 'Debrief.Activity',
      MessageTemplate: 'User {Operation} via {Trigger}',
      Operation: 'export.csv',
      Trigger: 'menu',
      Outcome: 'ok',
      DurationMs: '42',
      Window: 'main',
      View: 'Events',
      Title: 'Export CSV',
      Path: '<path>',
    });
    expect(props['OperationId']).toMatch(/^[0-9a-f-]{36}$/);
  });

  it('tracks errors (with a code) and cancellations, and rethrows', async () => {
    const { rt, out } = runtime();
    const boom = Object.assign(new Error('disk full'), { code: 'E_IO' });
    await expect(rt.track('export.csv', 'button', {}, () => Promise.reject(boom))).rejects.toBe(boom);
    expect(out[0]).toMatchObject({ level: 'warn', props: { Outcome: 'error', ErrorCode: 'E_IO' } });
    await expect(rt.track('file.open', 'dialog', {}, () => Promise.reject(new DOMException('x', 'AbortError')))).rejects.toBeTruthy();
    expect(out[1]?.props['Outcome']).toBe('cancelled');
    await expect(
      rt.track('sync', 'shortcut', {}, () => {
        throw new TypeError('bad');
      }),
    ).rejects.toBeInstanceOf(TypeError);
    expect(out[2]?.props).toMatchObject({ Outcome: 'error', ErrorCode: 'TypeError' });
  });

  it('describes errors', () => {
    expect(errorCode({ code: 'NOT_FOUND' })).toBe('NOT_FOUND');
    expect(errorCode('failed: E_PARSE at line 3')).toBe('E_PARSE');
    expect(errorCode(new RangeError('x'))).toBe('RangeError');
    expect(errorCode('plain')).toBe('E_FAILED');
    expect(errorCode(null)).toBe('E_FAILED');
    expect(isCancellation('Import cancelled')).toBe(true);
    expect(isCancellation({ code: 'CANCELLED' })).toBe(true);
    expect(isCancellation(new Error('broken'))).toBe(false);
  });

  it('keeps an operation current while it runs and briefly after', () => {
    const { rt, clock } = runtime();
    expect(rt.current()).toBeUndefined();
    const op = rt.begin('a', 'menu');
    expect(rt.currentOperationId()).toBe(op.id);
    rt.end(op, 'ok');
    rt.end(op, 'ok');
    expect(rt.currentOperationId()).toBe(op.id);
    clock.advance(LINGER_MS + 1);
    expect(rt.current()).toBeUndefined();
  });

  it('coalesces bursts into one entry with a count, current from the first change', () => {
    const { rt, clock, out } = runtime();
    const id = rt.coalesce('q', 'query.change', 'keyboard', { QueryLength: 1 }, 800);
    expect(rt.currentOperationId()).toBe(id);
    clock.advance(500);
    expect(rt.coalesce('q', 'query.change', 'keyboard', { QueryLength: 2 }, 800)).toBe(id);
    clock.advance(500);
    expect(out).toHaveLength(0);
    clock.advance(400);
    expect(out).toHaveLength(1);
    expect(out[0]?.props).toMatchObject({ Operation: 'query.change', QueryLength: '2', Count: '2', OperationId: id });
    rt.coalesce('t', 'timeRange.change', 'pointer', {}, 800);
    rt.flushCoalesced();
    expect(out[1]?.props['Operation']).toBe('timeRange.change');
    expect(out[1]?.props['Count']).toBeUndefined();
    // Bounded number of pending keys.
    for (let i = 0; i < 40; i++) rt.coalesce(`k${i}`, 'x', 'program', {}, 800);
    expect(out.length).toBeGreaterThan(2);
  });

  it('keeps a bounded ring of breadcrumbs and notifies listeners', () => {
    const { rt } = runtime();
    const seen: Breadcrumb[] = [];
    const off = rt.subscribe((b) => seen.push(b));
    rt.subscribe(() => {
      throw new Error('bad listener');
    });
    for (let i = 0; i < 250; i++) rt.record(`op${i}`, 'button', { I: i });
    off();
    expect(rt.breadcrumbs(1000)).toHaveLength(200);
    expect(rt.breadcrumbs(2).map((b) => b.operation)).toEqual(['op248', 'op249']);
    expect(seen).toHaveLength(250);
    expect(seen[0]).toMatchObject({ operation: 'op0', trigger: 'button', window: 'main', outcome: 'ok', params: { I: '0' } });
  });

  it('buffers entries until a sink is bound, and survives a throwing sink', () => {
    const rt = new ActivityRuntime(new FakeClock());
    rt.record('early', 'program');
    const got: string[] = [];
    rt.setSink((_l, m) => got.push(m));
    expect(got).toEqual(['User early via program']);
    rt.setSink(() => {
      throw new Error('sink down');
    });
    expect(() => rt.record('later', 'program')).not.toThrow();
    rt.setSink(null);
    rt.reset();
    expect(rt.breadcrumbs()).toEqual([]);
  });

  it('attributes a late native menu event', () => {
    const { rt, clock, out } = runtime();
    const op = rt.begin('file.open', 'auto');
    expect(op.trigger).toBe('program');
    rt.triggers.noteNativeMenu('file.open');
    clock.advance(5);
    rt.end(op, 'ok');
    expect(out[0]?.props['Trigger']).toBe('native-menu');
  });
});

describe('TriggerTracker', () => {
  afterEach(() => (document.body.innerHTML = ''));

  function el(html: string, selector: string): Element {
    document.body.innerHTML = html;
    return document.querySelector(selector)!;
  }

  it('classifies targets', () => {
    expect(classifyTarget(null)).toBe('unknown');
    expect(classifyTarget(el('<debrief-command-palette><div id="x"></div></debrief-command-palette>', '#x'))).toBe('palette');
    expect(classifyTarget(el('<debrief-title-bar><div role="menu"><button role="menuitem" id="x">Open</button></div></debrief-title-bar>', '#x'))).toBe('menu');
    expect(classifyTarget(el('<div role="menu"><button role="menuitem" id="x">Copy</button></div>', '#x'))).toBe('context-menu');
    expect(classifyTarget(el('<div role="menu" data-menu="File"><button role="menuitem" id="x">Open</button></div>', '#x'))).toBe('menu');
    expect(classifyTarget(el('<div role="dialog"><button id="x">OK</button></div>', '#x'))).toBe('dialog');
    expect(classifyTarget(el('<div><button id="x">Go</button></div>', '#x'))).toBe('button');
    expect(classifyTarget(el('<div><span id="x">row</span></div>', '#x'))).toBe('pointer');
  });

  it('classifies keys', () => {
    const key = (k: string, target: Element | null, mods: Partial<KeyboardEvent> = {}) => classifyKey({ key: k, ctrlKey: false, metaKey: false, altKey: false, target, ...mods });
    expect(key('p', null, { metaKey: true })).toBe('shortcut');
    expect(key('F5', null)).toBe('shortcut');
    expect(key('Enter', el('<debrief-command-palette><input id="x"></debrief-command-palette>', '#x'))).toBe('palette');
    expect(key('Enter', el('<div role="dialog"><input id="x"></div>', '#x'))).toBe('dialog');
    expect(key('a', el('<input id="x">', '#x'))).toBe('keyboard');
    expect(key('Escape', el('<div id="x"></div>', '#x'))).toBe('shortcut');
    expect(key('Enter', el('<div role="menu"><button role="menuitem" id="x"></button></div>', '#x'))).toBe('context-menu');
  });

  it('infers the trigger of an operation from recent DOM input', () => {
    let now = 0;
    const t = new TriggerTracker(() => now);
    const stop = t.install(document);
    document.body.innerHTML = '<button id="b">Go</button><div id="d"></div>';
    const b = document.getElementById('b')!;
    b.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: 0, clientY: 0 }));
    b.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    expect(t.infer()).toBe('button');
    expect(t.recentInput()).toBe(true);
    // A drag.
    const d = document.getElementById('d')!;
    d.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientX: 0, clientY: 0 }));
    d.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: 50, clientY: 0 }));
    d.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    expect(t.infer()).toBe('drag-drop');
    d.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
    expect(t.infer()).toBe('context-menu');
    b.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }));
    expect(t.infer()).toBe('button');
    d.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'k', ctrlKey: true }));
    expect(t.infer()).toBe('shortcut');
    b.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }));
    d.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'Shift' }));
    d.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'Tab' }));
    expect(t.infer()).toBe('button');
    d.dispatchEvent(new Event('drop', { bubbles: true }));
    expect(t.infer()).toBe('drag-drop');
    d.dispatchEvent(new Event('input', { bubbles: true }));
    expect(t.infer()).toBe('keyboard');
    now += 5000;
    expect(t.infer()).toBe('program');
    expect(t.recentInput()).toBe(false);
    t.noteNativeMenu('help.demo');
    expect(t.infer()).toBe('native-menu');
    stop();
    d.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'k', ctrlKey: true }));
    expect(t.infer()).toBe('native-menu');
  });
});

describe('ActivityLog', () => {
  beforeEach(() => activity.reset());

  it('sends entries to the logger under Debrief.Activity', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    const log = TestBed.inject(ActivityLog);
    log.setDetail('detailed');
    expect(log.detail).toBe('detailed');
    await log.track('help.demo', 'button', {}, () => undefined);
    log.record('dock.popout', 'button', { Panel: 'inspector' });
    log.coalesce('k', 'selection.change', 'pointer', { RecordId: 3 }, 10);
    activity.flushCoalesced();
    const entries = (TestBed.inject(Logger) as MemoryLogger).entries;
    expect(entries.map((e) => [e.category, e.message])).toEqual([
      ['Debrief.Activity', 'User help.demo via button'],
      ['Debrief.Activity', 'User dock.popout via button'],
      ['Debrief.Activity', 'User selection.change via pointer'],
    ]);
    expect(log.breadcrumbs(2).map((b) => b.operation)).toEqual(['dock.popout', 'selection.change']);
    expect(log.currentOperationId()).toBeTruthy();
  });
});

describe('IPC operation header', () => {
  it('adds the header without dropping others', () => {
    expect(withOpHeader(undefined, 'op1')).toEqual({ headers: { 'debrief-op-id': 'op1' } });
    expect(withOpHeader({ headers: { a: 'b' } }, 'op1')).toEqual({ headers: { 'debrief-op-id': 'op1', a: 'b' } });
    expect(withOpHeader({ headers: { 'debrief-op-id': 'mine' } }, 'op1').headers).toEqual({ 'debrief-op-id': 'mine' });
    expect(withOpHeader({ headers: [['a', 'b']] }, 'op1').headers).toEqual([
      ['a', 'b'],
      ['debrief-op-id', 'op1'],
    ]);
    expect(withOpHeader({ headers: [['Debrief-OP-ID', 'x']] }, 'op1').headers).toEqual([['Debrief-OP-ID', 'x']]);
    const h = withOpHeader({ headers: new Headers({ a: 'b' }) }, 'op1').headers as Headers;
    expect([h.get('a'), h.get('debrief-op-id')]).toEqual(['b', 'op1']);
  });
});

describe('steps to reproduce (mock port)', () => {
  const b = (operation: string, trigger: string, params: Record<string, string> = {}, extra: Partial<Breadcrumb> = {}): Breadcrumb => ({
    time: 't',
    operation,
    trigger,
    window: 'main',
    params,
    ...extra,
  });

  it('matches the engine golden journey', () => {
    // Same as `golden_journey` in app/src-tauri/src/activity/steps.rs.
    const crumbs = [
      b('file.open', 'drag-drop', { Path: '<dir#1a2b3c4d>/samples', Files: '3' }),
      b('query.change', 'unknown', { QueryHash: '9f86d081', QueryLength: '12' }),
      b('selection.change', 'unknown', { RecordId: '42' }),
      b('selection.change', 'unknown', { RecordId: '42' }),
      b('dock.popout', 'button', { Panel: 'Inspector' }),
      b('settings.change', 'dialog', { Section: 'diagnostics', Fields: 'logLevel' }),
      b('export.csv', 'menu', { Title: 'Export CSV' }, { outcome: 'error', errorCode: 'E_IO' }),
      b('file.export', 'dialog', { Format: 'csv' }, { outcome: 'cancelled' }),
    ];
    expect(steps(crumbs, 'Error E_IO: disk full')).toEqual([
      '1. Opened <dir#1a2b3c4d>/samples (3 files) by drag and drop',
      '2. Applied query (hash 9f86d081)',
      '3. Selected event 42 (×2)',
      '4. Popped out Inspector',
      '5. Changed settings diagnostics: logLevel in a dialog',
      "6. Clicked 'Export CSV' from the menu → error E_IO",
      '7. Exported csv events in a dialog (cancelled)',
      '8. Error E_IO: disk full',
    ]);
  });

  it('matches the engine fallbacks', () => {
    const crumbs = [
      b('query.change', 'palette', { Query: 'level:error', QueryHash: 'x' }),
      b('search.change', 'unknown', { SearchHash: 'abcd1234' }),
      b('ipc.save_profile', 'program', { Command: 'save_profile' }),
      b('ipc.reload', 'program'),
      b('plugin.thing', 'shortcut'),
      b('dock.close', 'shortcut'),
      b('timeRange.change', 'unknown'),
      b('filter.change', 'unknown', { Filters: '2' }),
      b('live.on', 'button'),
      b('edit.undo', 'shortcut'),
      b('file.open', 'native-menu'),
      b('file.save', 'context-menu', { File: 'p.debriefproj' }),
      b('dock.layout', 'menu'),
      b('ai.ask', 'button'),
    ];
    expect(steps(crumbs)).toEqual([
      '1. Applied query "level:error" from the command palette',
      '2. Searched for (hash abcd1234)',
      '3. Ran save_profile',
      '4. Ran reload',
      '5. Ran plugin.thing with a keyboard shortcut',
      '6. Closed a panel with a keyboard shortcut',
      '7. Changed the time range',
      '8. Changed the filters (2 active)',
      '9. Turned live tail on with a button',
      '10. Undid the last change with a keyboard shortcut',
      '11. Opened files from the app menu',
      '12. Saved p.debriefproj from a context menu',
      '13. Applied a panel layout from the menu',
      '14. Asked the AI assistant with a button',
    ]);
    expect(steps([b('x', 'unknown', {}, { outcome: 'error' })])).toEqual(['1. Ran x → error E_UNKNOWN']);
    const other = ['dock.open', 'dock.dockBack', 'dock.autoHide', 'live.off', 'ai.applyProposal', 'edit.redo', 'selection.change', 'settings.change'];
    expect(steps(other.map((o) => b(o, 'unknown'))).length).toBe(other.length);
    expect(steps([b('settings.change', 'x', { Section: 'ai' })])).toEqual(['1. Changed ai settings']);
    expect(steps([b('file.export', 'x', { Path: 'a.csv' }), b('file.save', 'x'), b('file.open', 'x', { Path: 'a' })])).toEqual([
      '1. Exported to a.csv',
      '2. Saved a file',
      '3. Opened a',
    ]);
  });

  it('cuts long histories', () => {
    const s = steps(Array.from({ length: 40 }, (_, i) => b(`op${i}`, 'unknown')));
    expect(s).toHaveLength(31);
    expect(s[0]).toBe('1. … (10 earlier steps)');
    expect(s[30]).toBe('31. Ran op39');
  });
});

describe('fault injection (test hook)', () => {
  it('replaces the named views with a throwing component, in browser builds only', async () => {
    const { faultyViews, withInjectedFaults, InjectedFault } = await import('./fault-injection');
    expect(faultyViews('?debriefFault=view:timeline', false)).toBe('timeline');
    expect(faultyViews('?debriefFault=view:*', true)).toBeNull();
    expect(faultyViews('?other=1', false)).toBeNull();
    const views = [{ id: 'a', title: 'A', icon: 'x', component: async () => Object }, { id: 'b', title: 'B', icon: 'x', component: async () => Object }];
    expect(withInjectedFaults(views, null)).toBe(views);
    const one = withInjectedFaults(views, 'b');
    expect(await one[0]?.component()).toBe(Object);
    expect(await one[1]?.component()).toBe(InjectedFault);
    expect(withInjectedFaults(views, '*').length).toBe(2);
    expect(() => new InjectedFault()).toThrow('Injected fault');
  });
});
