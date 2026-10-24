import { TestBed } from '@angular/core/testing';
import { Component } from '@angular/core';
import { settle } from '../../testing/helpers';
import { Logger } from '../core/logger';
import { DIALOG_BOUNDS_KEY, rememberDialogBounds } from './dialog-bounds';
import { DIALOG_IO } from './dialog-io';
import { FakeDialogIo } from './dialog-io-fake';
import { DialogLabels } from './dialog-labels';
import { dialogDef } from './dialog-registry';
import { DialogShowRequest } from './dialog-session';
import { InPageDialogBackend } from './in-page-dialog-backend';
import { parseClosed, parseResult, TauriDialogBackend } from './tauri-dialog-backend';

@Component({ selector: 'debrief-t', template: '' })
class T {
  static readonly dialogId = 'settings';
}

const warn = vi.fn();

function setup() {
  const io = new FakeDialogIo();
  TestBed.configureTestingModule({
    providers: [
      { provide: DIALOG_IO, useValue: io },
      { provide: Logger, useValue: { debug: vi.fn(), info: vi.fn(), warn, error: vi.fn() } },
    ],
  });
  return { io, backend: TestBed.inject(TauriDialogBackend), inPage: TestBed.inject(InPageDialogBackend), labels: TestBed.inject(DialogLabels) };
}

const request = (id = 'settings', inputs: Record<string, unknown> = { section: 'x' }): DialogShowRequest => ({ def: dialogDef(id)!, component: T, title: 'Settings', inputs, width: 900 });
const tick = () => new Promise((r) => setTimeout(r));

describe('TauriDialogBackend', () => {
  beforeEach(() => {
    localStorage.removeItem(DIALOG_BOUNDS_KEY);
    warn.mockClear();
  });

  it('opens a window with the typed input, modality and size, and resolves with the dialog result', async () => {
    const { io, backend, labels } = setup();
    expect(backend.windowed).toBe(true);
    const s = backend.show(request());
    await tick();
    expect(io.opened).toHaveLength(1);
    expect(io.opened[0]).toMatchObject({ dialogId: 'settings', title: 'Settings', input: { section: 'x' }, width: 900, remembered: undefined });
    expect(labels.labels()).toEqual(['d-main-1']);
    io.result('d-main-1', 'settings', { ok: 1 });
    io.closed('d-main-1', 'settings');
    expect(await s.closed).toEqual({ ok: 1 });
    expect(labels.labels()).toEqual([]);
  });

  it('leaves modality and the required choice to the engine, which reads them from the dialog contract', async () => {
    const { io, backend } = setup();
    backend.show(request('close-prompt'));
    await tick();
    expect(io.opened[0]).toMatchObject({ dialogId: 'close-prompt' });
    expect(Object.keys(io.opened[0]!)).not.toContain('modality');
    expect(Object.keys(io.opened[0]!)).not.toContain('requiredChoice');
  });

  it('raises a remembered viewer size below 680x560 to the floor of its kind', async () => {
    const { io, backend } = setup();
    rememberDialogBounds('ai-narrative', { dx: 10, dy: 10, width: 400, height: 300 });
    backend.show(request('ai-narrative'));
    await tick();
    expect(io.opened[0]!.remembered).toEqual({ dx: 10, dy: 10, width: 680, height: 560 });
  });

  it('resolves undefined when the window closed without a result, and ignores events of other windows and malformed ones', async () => {
    const { io, backend } = setup();
    const s = backend.show(request('ai-setup'));
    await tick();
    io.result('d-main-9', 'ai-setup', 'other');
    io.emit('debrief://dialog-closed', { nonsense: true });
    io.emit('debrief://dialog-result', 5);
    io.closed('d-main-9', 'ai-setup');
    io.closed('d-main-1', 'ai-setup');
    expect(await s.closed).toBeUndefined();
    // Rust takes the modality and the required choice from the dialog's contract, not from the webview's request.
    expect(Object.keys(io.opened[0]!)).not.toContain('modality');
    expect(Object.keys(io.opened[0]!)).not.toContain('requiredChoice');
  });

  it('remembers the size and the offset from the parent the shell reported, and sends them back as they are', async () => {
    const { io, backend } = setup();
    const first = backend.show(request());
    await tick();
    io.closed('d-main-1', 'settings', { dx: 40, dy: -20, width: 800, height: 600 });
    await first.closed;
    expect(JSON.parse(localStorage.getItem(DIALOG_BOUNDS_KEY)!).bounds.settings).toEqual({ dx: 40, dy: -20, width: 800, height: 600 });
    backend.show(request());
    await tick();
    // Never an absolute position: the shell keeps the dialog on its parent's monitor (ADR 0054, placement policy).
    expect(io.opened[1]!.remembered).toEqual({ dx: 40, dy: -20, width: 800, height: 600 });
    expect(Object.keys(io.opened[1]!)).not.toContain('bounds');
  });

  it('focuses the existing window by asking to open it again, and close asks the dialog to close', async () => {
    const { io, backend } = setup();
    const s = backend.show(request());
    await tick();
    s.focus();
    await tick();
    expect(io.opened).toHaveLength(2);
    s.close();
    await tick();
    expect(io.closeRequests).toEqual(['d-main-1']);
  });

  it('shows the dialog in the page when no window can be opened (degrade, do not fail)', async () => {
    const { io, backend, inPage } = setup();
    io.failOpen = new Error('too many windows');
    const s = backend.show(request());
    await tick();
    expect(inPage.stack()).toHaveLength(1);
    expect(warn).toHaveBeenCalled();
    s.focus();
    inPage.stack()[0]!.ref.close('r');
    expect(await s.closed).toBe('r');
    // close on a fallback closes the in-page dialog
    const s2 = backend.show(request());
    await tick();
    s2.close('z');
    await tick();
    expect(await s2.closed).toBe('z');
  });

  it('shows overlays (no definition) in the page', () => {
    const { io, backend, inPage } = setup();
    backend.show({ def: null, component: T, title: 'Palette', inputs: {}, width: 500 });
    expect(inPage.stack()).toHaveLength(1);
    expect(io.opened).toHaveLength(0);
  });

  it('asks yes/no questions with the native message box; a failing box counts as No', async () => {
    const { io, backend } = setup();
    expect(await backend.confirm({ title: 't', message: 'm', ok: 'Yes', cancel: 'No', danger: true })).toBe(true);
    expect(io.asked[0]!.danger).toBe(true);
    io.ask = () => Promise.reject(new Error('no box'));
    expect(await backend.confirm({ title: 't', message: 'm', ok: 'Yes', cancel: 'No', danger: false })).toBe(false);
    await settle(TestBed.createComponent(T));
  });

  it('parses the shell events strictly', () => {
    expect(parseResult({ label: 'd-main-1', dialogId: 'a', result: 3 })).toEqual({ label: 'd-main-1', dialogId: 'a', result: 3 });
    expect(parseResult({ label: 1 })).toBeNull();
    expect(parseResult([])).toBeNull();
    expect(parseClosed({ label: 'l', dialogId: 'a', remembered: { dx: 1, dy: -2, width: 3, height: 4, extra: 1 } })?.remembered).toEqual({ dx: 1, dy: -2, width: 3, height: 4 });
    expect(parseClosed({ label: 'l', dialogId: 'a', remembered: { dx: 'q' } })?.remembered).toBeNull();
    expect(parseClosed({ label: 'l', dialogId: 'a', bounds: { x: 1, y: 2, width: 3, height: 4 } })?.remembered).toBeNull();
    expect(parseClosed({ label: 'l' })).toBeNull();
    expect(parseClosed(null)).toBeNull();
  });
});
