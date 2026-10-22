import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../../testing/helpers';
import { DialogService } from '../../ui/dialog.service';
import { PasteCommands } from './paste-commands';
import { droppedText, dragsText, startTextDrops } from './paste-drop';

describe('Paste as log command', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: [provideTinyBackend()] }));

  it('is registered as edit.pasteLog with Mod+Shift+V and opens the dialog', async () => {
    const cmd = TestBed.inject(PasteCommands).commands[0]!;
    expect(cmd).toMatchObject({ id: 'edit.pasteLog', title: 'Paste as log…', keybinding: 'Mod+Shift+V' });
    await cmd.run({} as never);
    const open = TestBed.inject(DialogService).stack();
    expect(open).toHaveLength(1);
    expect(open[0]?.options.title).toBe('Paste as log');
    expect(open[0]?.options.inputs).toEqual({ text: null });
  });

  it('opens with the dropped text', async () => {
    await TestBed.inject(PasteCommands).open('dropped');
    expect(TestBed.inject(DialogService).stack()[0]?.options.inputs).toEqual({ text: 'dropped' });
  });
});

describe('dropped text', () => {
  const data = (files: number, types: string[], plain: string) => ({ files: { length: files }, types, getData: () => plain });

  it('is plain text without files', () => {
    expect(droppedText(data(0, ['text/plain'], 'a b'))).toBe('a b');
    expect(droppedText(data(1, ['text/plain'], 'a b'))).toBeNull();
    // An event dragged out of the grid carries its text, but is not text to paste.
    expect(droppedText(data(0, ['text/plain', 'application/x-imt-event'], 'a b'))).toBeNull();
    expect(dragsText({ files: { length: 0 }, types: ['text/plain', 'application/x-imt-event'] })).toBe(false);
    expect(droppedText(data(0, ['text/plain'], '  \n'))).toBeNull();
    expect(droppedText(null)).toBeNull();
    expect(dragsText(data(0, ['text/plain'], ''))).toBe(true);
    expect(dragsText(data(0, ['text/html'], ''))).toBe(false);
    expect(dragsText(data(2, ['text/plain'], ''))).toBe(false);
    expect(dragsText(null)).toBe(false);
  });

  it('opens the dialog when text is dropped on the page, not when on a text field or with files', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    TestBed.runInInjectionContext(() => startTextDrops());
    const open = vi.spyOn(TestBed.inject(PasteCommands), 'open').mockResolvedValue();
    const fire = (type: string, target: EventTarget, dt: object) => {
      const e = new Event(type, { bubbles: true, cancelable: true });
      Object.defineProperty(e, 'dataTransfer', { value: dt });
      target.dispatchEvent(e);
      return e;
    };
    const dt = { files: { length: 0 }, types: ['text/plain'], getData: () => 'some log' };
    expect(fire('dragover', document.body, dt).defaultPrevented).toBe(true);
    expect(fire('drop', document.body, dt).defaultPrevented).toBe(true);
    expect(open).toHaveBeenCalledWith('some log');
    open.mockClear();
    const input = document.body.appendChild(document.createElement('input'));
    expect(fire('dragover', input, dt).defaultPrevented).toBe(false);
    fire('drop', input, dt);
    fire('drop', document.body, { ...dt, files: { length: 1 } });
    expect(open).not.toHaveBeenCalled();
    input.remove();
  });
});
