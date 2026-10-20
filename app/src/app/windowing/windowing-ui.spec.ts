import { ApplicationInitStatus } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { provideToolsTesting } from '../tools/mock/test-support';
import { DialogService } from '../ui/dialog.service';
import { DialogHost } from '../ui/dialog-host';
import { BackgroundWork } from './background-work';
import { ClosePromptService } from './close-prompt';
import { memoryHost, MockWindowingBackend } from './mock-windowing-backend';
import { OpenAskService, openAskTitle } from './open-ask-dialog';
import { provideWindowing } from './provide-windowing';
import { WindowingBackend } from './windowing-backend';
import { WindowingService } from './windowing.service';

function setup(href = 'http://localhost/?mock-tasks') {
  const backend = new MockWindowingBackend(memoryHost(href));
  TestBed.configureTestingModule({ providers: [...provideTinyBackend(), { provide: WindowingBackend, useValue: backend }] });
  return { backend, store: TestBed.inject(WorkspaceStore) };
}

async function dialogHost() {
  const fixture = TestBed.createComponent(DialogHost);
  await settle(fixture);
  return fixture;
}

describe('Close prompt', () => {
  it('quits or keeps running in the tray, remembering the choice by default', async () => {
    const { backend } = setup();
    const fixture = await dialogHost();
    const ref = TestBed.inject(ClosePromptService).open();
    await settle(fixture);
    expect(el(fixture, '.dialog h2').textContent).toBe('Close IMT?');
    expect(text(fixture)).toContain('keeps running in the tray');
    expect(el<HTMLInputElement>(fixture, 'input[type=checkbox]').checked).toBe(true);
    buttonByText(fixture, 'Keep running in tray').click();
    expect(await ref.closed).toBe('tray');
    expect(backend.calls).toContain('resolveClose:tray:true');

    const again = TestBed.inject(ClosePromptService).open();
    await settle(fixture);
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    await settle(fixture);
    buttonByText(fixture, 'Quit').click();
    expect(await again.closed).toBe('quit');
    expect(backend.calls).toContain('resolveClose:quit:false');
  });

  it('does nothing when dismissed, and shows a failure without closing', async () => {
    const { backend, store } = setup();
    const fixture = await dialogHost();
    const dialogs = TestBed.inject(DialogService);
    const ref = TestBed.inject(ClosePromptService).open();
    dialogs.dismissTop();
    expect(await ref.closed).toBeUndefined();
    expect(backend.calls.filter((c) => c.startsWith('resolveClose'))).toEqual([]);

    vi.spyOn(backend, 'resolveClose').mockRejectedValue('busy');
    TestBed.inject(ClosePromptService).open();
    await settle(fixture);
    buttonByText(fixture, 'Quit').click();
    await settle(fixture);
    expect(store.error()).toBe('Could not close the window: busy');
    expect(dialogs.stack()).toHaveLength(1);
  });
});

describe('Open-ask dialog', () => {
  it('titles the count and lists the names', () => {
    expect(openAskTitle(1)).toBe('Open 1 file');
    expect(openAskTitle(3)).toBe('Open 3 files');
  });

  it('opens in this window or a new one', async () => {
    const { backend } = setup();
    const fixture = await dialogHost();
    const ref = TestBed.inject(OpenAskService).open({ count: 2, names: ['a.log', 'b.log'] });
    await settle(fixture);
    expect(all(fixture, '[aria-label="Files to open"] li').map((li) => li.textContent?.trim())).toEqual(['a.log', 'b.log']);
    expect(text(fixture)).toContain('these files');
    buttonByText(fixture, 'New window').click();
    expect(await ref.closed).toBe('newWindow');

    const one = TestBed.inject(OpenAskService).open({ count: 1, names: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('this file');
    expect(all(fixture, '[aria-label="Files to open"]')).toHaveLength(0);
    buttonByText(fixture, 'This window').click();
    expect(await one.closed).toBe('here');
    await settle(fixture);
    expect(backend.calls).toEqual(['resolveOpen:newWindow', 'resolveOpen:here']);
  });

  it('drops the waiting files when dismissed by Escape, the close button or the backdrop', async () => {
    const { backend } = setup();
    const fixture = await dialogHost();
    const dialogs = TestBed.inject(DialogService);
    const service = TestBed.inject(OpenAskService);
    const cancels = () => backend.calls.filter((c) => c === 'resolveOpen:cancel').length;

    const byEscape = service.open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(await byEscape.closed).toBeUndefined();
    await settle(fixture);
    expect(cancels()).toBe(1);

    const byButton = service.open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '.dialog button[aria-label="Close dialog"]').click();
    expect(await byButton.closed).toBeUndefined();
    await settle(fixture);
    expect(cancels()).toBe(2);

    const byBackdrop = service.open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    el(fixture, '.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(await byBackdrop.closed).toBeUndefined();
    await settle(fixture);
    expect(cancels()).toBe(3);
    expect(dialogs.stack()).toHaveLength(0);
    expect(backend.calls).toEqual(['resolveOpen:cancel', 'resolveOpen:cancel', 'resolveOpen:cancel']);
  });

  it('does not cancel after a choice, and reports a failed cancel', async () => {
    const { backend, store } = setup();
    const fixture = await dialogHost();
    const ref = TestBed.inject(OpenAskService).open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    buttonByText(fixture, 'This window').click();
    await ref.closed;
    await settle(fixture);
    expect(backend.calls).toEqual(['resolveOpen:here']);

    vi.spyOn(backend, 'resolveOpen').mockRejectedValue('window gone');
    TestBed.inject(OpenAskService).open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    TestBed.inject(DialogService).dismissTop();
    await settle(fixture);
    expect(store.error()).toBe('Could not cancel opening the files: window gone');
  });

  it('shows a failure and stays open', async () => {
    const { backend, store } = setup();
    const fixture = await dialogHost();
    vi.spyOn(backend, 'resolveOpen').mockRejectedValue(new Error('gone'));
    TestBed.inject(OpenAskService).open({ count: 1, names: ['a.log'] });
    await settle(fixture);
    buttonByText(fixture, 'This window').click();
    await settle(fixture);
    expect(store.error()).toBe('Could not open the files: gone');
    expect(TestBed.inject(DialogService).stack()).toHaveLength(1);
  });
});

describe('BackgroundWork', () => {
  async function render(href?: string) {
    const ctx = setup(href);
    await TestBed.inject(WindowingService).init();
    const fixture = TestBed.createComponent(BackgroundWork);
    await settle(fixture);
    return { ...ctx, fixture };
  }

  it('is hidden when nothing runs in the background', async () => {
    const { fixture } = await render('http://localhost/');
    expect(all(fixture, 'button')).toHaveLength(0);
  });

  it('shows the count, lists tasks with progress and stops them', async () => {
    const { fixture, backend } = await render();
    const trigger = el<HTMLButtonElement>(fixture, 'button[aria-label="Background work"]');
    expect(trigger.textContent?.trim()).toBe('3');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    trigger.click();
    await settle(fixture);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(el(fixture, '[role=dialog]').textContent).toContain('Live tail — main');
    expect(all(fixture, 'progress')).toHaveLength(1);
    expect(el<HTMLButtonElement>(fixture, 'button[aria-label="Stop Downloading symbols"]').disabled).toBe(true);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Stop Process monitor"]').click();
    await settle(fixture);
    expect(backend.calls).toContain('stopBackgroundTask:monitor');
    expect(trigger.textContent?.trim()).toBe('2');
    expect(all(fixture, '[role=dialog] li')).toHaveLength(2);

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await settle(fixture);
    expect(all(fixture, '[role=dialog]')).toHaveLength(0);
  });

  it('closes the popover when the last task stops', async () => {
    const { fixture, backend } = await render();
    vi.spyOn(backend, 'stopBackgroundTask').mockResolvedValue();
    const service = TestBed.inject(WindowingService);
    service.tasks.set(service.tasks().filter((t) => t.id === 'monitor'));
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Background work"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Stop Process monitor"]').click();
    await settle(fixture);
    expect(all(fixture, 'button')).toHaveLength(0);
    service.tasks.set([{ id: 'x', label: 'X', kind: 'other', window: null, progress: null, stoppable: true, startedAt: 0 }]);
    await settle(fixture);
    expect(all(fixture, '[role=dialog]')).toHaveLength(0);
  });
});

describe('provideWindowing', () => {
  it('registers the commands and starts the service', async () => {
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), provideWindowing()] });
    await TestBed.inject(ApplicationInitStatus).donePromise;
    expect(TestBed.inject(CommandRegistry).find('window.new')?.title).toBe('New window');
    await settle();
    expect(TestBed.inject(WindowingService).windows().map((w) => w.label)).toEqual(['main']);
  });
});
