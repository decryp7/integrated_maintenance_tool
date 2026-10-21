import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { Platform } from '../../core/platform';
import { MockJobs, resetMockJobs } from '../../jobs/mock-jobs';
import { all, buttonByText, el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiStatusStore } from '../ai-status.store';
import { AiImageStore } from '../vision/ai-image.store';
import { AiChatPanel } from './chat-panel';
import { ChatStore } from './chat.store';

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13]);

function setup(options: MockAiOptions = {}, pick: string | null = '/shots/dialog.png') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const platform = Object.assign(new Platform(), { pickOpenPath: vi.fn(async () => pick) });
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: Platform, useValue: platform }],
  });
  const f = TestBed.createComponent(AiChatPanel);
  const store = TestBed.inject(ChatStore);
  return { backend, platform, f, store, images: TestBed.inject(AiImageStore), root: f.nativeElement as HTMLElement };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

async function ready(env: ReturnType<typeof setup>) {
  env.f.detectChanges();
  await until(env.f, () => !!env.store.activeId() && !el<HTMLTextAreaElement>(env.f, 'textarea').disabled);
}

/** A paste or drop event carrying `files` (jsdom has no DataTransfer). */
function withFiles(type: 'paste' | 'drop' | 'dragover', files: File[], types = ['Files']): Event {
  const e = new Event(type, { bubbles: true, cancelable: true });
  const data = { files, items: [], types };
  Object.defineProperty(e, type === 'paste' ? 'clipboardData' : 'dataTransfer', { value: data });
  return e;
}

const png = (name = 'shot.png') => new File([PNG], name, { type: 'image/png' });

describe('AiChatPanel images', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('attaches a picked image, sends it with the question and shows it with the turn', async () => {
    const env = setup();
    const { f, root, backend, platform, store } = env;
    await ready(env);
    el<HTMLButtonElement>(f, '[data-testid=ai-attach]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-attachment]'));
    expect(platform.pickOpenPath).toHaveBeenCalledWith('Attach an image', [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'gif', 'webp'] }]);
    const chip = el(f, '[data-testid=ai-attachment]');
    expect(chip.textContent).toContain('dialog.png');
    expect(chip.querySelector('img')?.getAttribute('src')).toMatch(/^data:image\//);
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), 'what does this dialog say?');
    f.detectChanges();
    el<HTMLButtonElement>(f, 'button[type=submit]').click();
    await until(f, () => !store.running() && !!store.activeTurns()[0]?.text);
    expect(backend.sent.at(-1)?.images).toEqual([store.imagesOf(0)[0]!.id]);
    expect(root.querySelector('[data-testid=ai-attachment]')).toBeNull();
    const thumbs = all<HTMLImageElement>(f, '[data-testid=ai-turn-images] img');
    expect(thumbs.map((t) => t.alt)).toEqual(['dialog.png']);
    expect(el(f, '[data-testid=ai-answer-0]').textContent).toContain('error dialog');
    // A question without images sends none.
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), 'and now?');
    f.detectChanges();
    el<HTMLButtonElement>(f, 'button[type=submit]').click();
    await until(f, () => !store.running() && store.activeTurns().length === 2 && !!store.activeTurns()[1]?.text);
    expect(backend.sent.at(-1)?.images).toEqual([]);
    expect(store.imagesOf(1)).toEqual([]);
  });

  it('attaches pasted and dropped images as bytes, and leaves text pastes alone', async () => {
    const env = setup();
    const { f, root, backend } = env;
    await ready(env);
    const paste = vi.spyOn(backend, 'imagePaste');
    const ta = el<HTMLTextAreaElement>(f, 'textarea');
    const text = withFiles('paste', []);
    ta.dispatchEvent(text);
    expect(text.defaultPrevented).toBe(false);
    const ev = withFiles('paste', [png('clip.png')]);
    ta.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    await until(f, () => all(f, '[data-testid=ai-attachment]').length === 1);
    expect(paste).toHaveBeenCalledWith('iVBORw0KGgoAAAAN', 'clip.png');
    const form = el<HTMLFormElement>(f, 'form.composer');
    const over = withFiles('dragover', []);
    form.dispatchEvent(over);
    f.detectChanges();
    expect(over.defaultPrevented).toBe(true);
    expect(form.classList).toContain('dropping');
    form.dispatchEvent(new Event('dragleave'));
    f.detectChanges();
    expect(form.classList).not.toContain('dropping');
    form.dispatchEvent(withFiles('dragover', [], ['text/plain']));
    const drop = withFiles('drop', [png('drop.png'), new File(['x'], 'notes.txt', { type: 'text/plain' })]);
    form.dispatchEvent(drop);
    expect(drop.defaultPrevented).toBe(true);
    await until(f, () => all(f, '[data-testid=ai-attachment]').length === 2);
    const notImage = withFiles('drop', [new File(['x'], 'notes.txt', { type: 'text/plain' })]);
    form.dispatchEvent(notImage);
    f.detectChanges();
    expect(notImage.defaultPrevented).toBe(true);
    expect(root.textContent).toContain('Only PNG, JPEG, BMP, GIF and WebP images can be attached.');
    // Not a file drag: the browser handles it.
    const textDrop = withFiles('drop', [], ['text/plain']);
    form.dispatchEvent(textDrop);
    expect(textDrop.defaultPrevented).toBe(false);
    // Removing a chip detaches the image.
    el<HTMLButtonElement>(f, 'button[aria-label="Remove clip.png"]').click();
    f.detectChanges();
    expect(all(f, '[data-testid=ai-attachment]').map((c) => c.textContent)).toEqual([expect.stringContaining('drop.png')]);
  });

  it('takes at most 4 images per question', async () => {
    const env = setup();
    const { f, root, store } = env;
    await ready(env);
    el<HTMLTextAreaElement>(f, 'textarea').dispatchEvent(withFiles('paste', [png('1.png'), png('2.png'), png('3.png'), png('4.png'), png('5.png')]));
    await until(f, () => all(f, '[data-testid=ai-attachment]').length === 4);
    expect(root.textContent).toContain('At most 4 images go with one question.');
    expect(el<HTMLButtonElement>(f, '[data-testid=ai-attach]').disabled).toBe(true);
    store.setNotice(null);
    const fifth = (await TestBed.inject(AiImageStore).openPath('/x/5.png', false))!;
    expect(store.attach(fifth)).toBe(false);
    expect(store.attach(store.attachments()[0]!)).toBe(true);
    expect(store.notice()).toContain('At most 4');
  });

  it('asks for the logs related to one attachment from its chip', async () => {
    const env = setup();
    const { f, store, backend } = env;
    await ready(env);
    el<HTMLButtonElement>(f, '[data-testid=ai-attach]').click();
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-attachment]'));
    buttonByText(f, 'Find related logs').click();
    await until(f, () => !store.running() && !!store.activeTurns()[0]?.text);
    expect(backend.sent.at(-1)?.text).toBe('find the logs related to this screenshot');
    expect(backend.sent.at(-1)?.images.length).toBe(1);
    const answer = el(f, '[data-testid=ai-answer-0]');
    expect(answer.textContent).toContain('Linked events:');
    expect(answer.querySelectorAll('button.cite.rec').length).toBeGreaterThan(0);
    expect(store.attachments()).toEqual([]);
  });

  it('shows the no-vision offer instead of attaching to a text-only model', async () => {
    const env = setup({ vision: 'missing' });
    const { f, root, platform, images } = env;
    await ready(env);
    await TestBed.inject(AiStatusStore).refresh();
    el<HTMLButtonElement>(f, '[data-testid=ai-attach]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-no-vision]'));
    expect(platform.pickOpenPath).not.toHaveBeenCalled();
    await until(f, () => !!root.querySelector('[data-testid=ai-add-image-support]'));
    expect(el(f, '[data-testid=ai-add-image-support]').textContent).toContain('640 MB');
    images.dismissNoVision();
    el<HTMLTextAreaElement>(f, 'textarea').dispatchEvent(withFiles('paste', [png()]));
    await until(f, () => images.noVision());
    expect(env.store.attachments()).toEqual([]);
  });

  it('does not send images to a text-only model, and turns its refusal into the offer', async () => {
    const env = setup();
    const { store, backend, images } = env;
    await ready(env);
    const img = (await images.openPath('/shots/a.png', false))!;
    store.attach(img);
    const status = TestBed.inject(AiStatusStore);
    status.setStatus({ ...status.status()!, vision: false });
    const send = vi.spyOn(backend, 'chatSend');
    expect(await store.send('what is it?')).toBe(false);
    expect(send).not.toHaveBeenCalled();
    expect(images.noVision()).toBe(true);
    images.dismissNoVision();
    status.setStatus({ ...status.status()!, vision: true });
    send.mockRejectedValueOnce(new AiError('noVision', 'text only'));
    expect(await store.send('what is it?')).toBe(false);
    expect(images.noVision()).toBe(true);
    expect(store.error()).toBeNull();
    expect(store.attachments().length).toBe(1);
    store.detach(img.id);
    expect(store.attachments()).toEqual([]);
  });

  it('shows a failed attach inline with Retry', async () => {
    const env = setup({}, '/shots/notes.txt');
    const { f, root, images } = env;
    await ready(env);
    el<HTMLButtonElement>(f, '[data-testid=ai-attach]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-attach-error]'));
    expect(el(f, '[data-testid=ai-attach-error]').textContent).toContain('PNG, JPEG');
    expect(images.openError()).toBeNull();
    const open = vi.spyOn(images, 'openPath').mockResolvedValueOnce((await TestBed.inject(AiBackend).imageOpen('/shots/ok.png')));
    buttonByText(f, 'Retry').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-attachment]'));
    expect(open).toHaveBeenCalledWith('/shots/notes.txt', false);
    expect(root.querySelector('[data-testid=ai-attach-error]')).toBeNull();
    el<HTMLTextAreaElement>(f, 'textarea').dispatchEvent(withFiles('paste', [new File([new Uint8Array([1, 2])], 'bad.png', { type: 'image/png' })]));
    await until(f, () => !!root.querySelector('[data-testid=ai-attach-error]'));
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-attach-error]')).toBeNull();
    // A cancelled picker attaches nothing.
    (env.platform.pickOpenPath as ReturnType<typeof vi.fn>).mockResolvedValueOnce(null);
    el<HTMLButtonElement>(f, '[data-testid=ai-attach]').click();
    await settle(f);
    expect(all(f, '[data-testid=ai-attachment]').length).toBe(1);
  });
});
