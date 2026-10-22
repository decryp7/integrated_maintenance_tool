import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { TimeCursor } from '../../analysis/time-cursor';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { mockRelatedOutcome } from '../../core/mock/mock-ai-vision';
import { Platform } from '../../core/platform';
import { MockJobs, mockJobs, resetMockJobs } from '../../jobs/mock-jobs';
import { ClockDialogService } from '../../skew/clock-section';
import { TimelineZoom } from '../../timeline/zoom';
import { all, buttonByText, el, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiImage, RelatedOutcome } from '../ai.types';
import { ChatStore } from '../chat/chat.store';
import { AiCitations } from '../chat/citation-actions';
import { AiImageStore } from './ai-image.store';
import { AiImagePanel } from './image-panel';
import { ImageViewer } from './image-viewer';
import { AiNoVisionCard } from './no-vision-card';
import { AiRelatedResults } from './related-results';

function setup(options: MockAiOptions = {}, pick: string | null = '/shots/dialog.png') {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  const ctx = new FakeContext();
  const platform = Object.assign(new Platform(), { pickOpenPath: vi.fn(async () => pick) });
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: ctx }), { provide: AiBackend, useValue: backend }, { provide: Platform, useValue: platform }],
  });
  return { backend, ctx, platform };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

const image: AiImage = {
  id: 3,
  name: 'dialog.png',
  path: '/shots/dialog.png',
  width: 1366,
  height: 768,
  originalWidth: 2732,
  originalHeight: 1536,
  sourceFormat: 'jpeg',
  dataUrl: 'data:image/png;base64,iVBORw0KGgo=',
  capturedAt: '2026-10-14 10:41:26',
};

describe('ImageViewer', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the clean copy fit to the panel or at 100 %, its sizes and capture time', () => {
    const f = TestBed.createComponent(ImageViewer);
    f.componentRef.setInput('image', image);
    f.detectChanges();
    const img = el<HTMLImageElement>(f, 'img');
    expect(img.getAttribute('src')).toBe(image.dataUrl);
    expect(img.alt).toBe('dialog.png');
    expect(img.getAttribute('width')).toBeNull();
    expect(el(f, '[data-testid=image-facts]').textContent).toContain('1366 × 768 · original 2732 × 1536 JPEG');
    expect(el(f, '[data-testid=image-facts]').textContent).toContain('Captured 2026-10-14 10:41:26');
    const zoom = el<HTMLButtonElement>(f, '[data-testid=image-zoom]');
    expect(zoom.textContent).toContain('100 %');
    zoom.click();
    f.detectChanges();
    expect(zoom.getAttribute('aria-pressed')).toBe('true');
    expect(zoom.textContent).toContain('Fit');
    expect(img.getAttribute('width')).toBe('1366');
    expect(el(f, '.frame').classList).toContain('actual');
    f.componentRef.setInput('image', { ...image, capturedAt: null });
    f.detectChanges();
    expect(el(f, '[data-testid=image-facts]').textContent).not.toContain('Captured');
  });
});

describe('AiRelatedResults', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  function render(outcome: RelatedOutcome) {
    const env = setup();
    const f = TestBed.createComponent(AiRelatedResults);
    f.componentRef.setInput('outcome', outcome);
    f.detectChanges();
    return { ...env, f };
  }

  it('lists the linked events with their reasons, and opens them in the grid and on the timeline', async () => {
    const o = mockRelatedOutcome('dialog.png');
    const { f, ctx } = render(o);
    const rows = all(f, '[data-testid=ai-related-link]');
    expect(rows.length).toBe(o.result.links.length);
    expect(rows[0]!.textContent).toContain('Fatal');
    const chips = [...rows[0]!.querySelectorAll('[data-testid=ai-related-reason]')].map((c) => c.textContent?.trim());
    expect(chips).toContain('time -1.4 s');
    expect(chips).toContain('title matches Application OrderService');
    expect(chips.some((c) => c?.startsWith("text '"))).toBe(true);
    expect(el(f, '[data-testid=ai-related-time]').textContent).toContain('shown on the screen');
    expect(el(f, '[data-testid=ai-related]').textContent).toContain('Error dialog');
    const record = vi.spyOn(TestBed.inject(AiCitations), 'record').mockResolvedValue();
    (rows[0]!.querySelector('button') as HTMLButtonElement).click();
    expect(record).toHaveBeenCalledWith(o.result.links[0]!.event.id);
    const zoom = vi.spyOn(TestBed.inject(TimelineZoom), 'apply');
    [...rows[0]!.querySelectorAll('button')].find((b) => b.textContent?.includes('Show on timeline'))!.click();
    const ts = o.result.links[0]!.event.ts!;
    expect(zoom).toHaveBeenCalledWith({ from: ts - 60_000, to: ts + 60_000 });
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: ts - 500, to: ts + 500 });
    expect(ctx.last('showView')).toEqual(['timeline']);
    // Citations in the explanation work like the chat's.
    const cite = el<HTMLButtonElement>(f, 'imt-ai-rich-text button.cite.rec');
    cite.click();
    expect(record).toHaveBeenCalledTimes(2);
  });

  it('offers the clock correction when the clocks disagree, and says when there was no time', () => {
    const o = mockRelatedOutcome('clock-skew.png');
    const { f } = render(o);
    const open = vi.spyOn(TestBed.inject(ClockDialogService), 'open').mockImplementation(() => undefined);
    expect(el(f, '[data-testid=ai-related-note-clockDisagreement]').textContent).toContain('The clocks likely disagree');
    el<HTMLButtonElement>(f, '[data-testid=ai-related-clock]').click();
    const note = o.result.notes.find((n) => n.kind === 'clockDisagreement');
    expect(open).toHaveBeenCalledWith(note && 'sourceId' in note ? note.sourceId : -1, 'orders-20261014.clef');
    TestBed.resetTestingModule();

    const none = render(mockRelatedOutcome('console.png'));
    expect(el(none.f, '[data-testid=ai-related-note-noTime]').textContent).toContain('matched by text and title only');
    expect(none.f.nativeElement.querySelector('[data-testid=ai-related-time]')).toBeNull();
  });

  it('says when nothing matched, and shows why a cited query was not applied', async () => {
    const o = mockRelatedOutcome('dialog.png');
    const empty: RelatedOutcome = { ...o, result: { ...o.result, links: [] }, explanation: { ...o.explanation, text: 'See `level >= error`.', citations: [{ kind: 'query', ql: 'level >= error' }] } };
    const { f } = render(empty);
    expect(el(f, '[data-testid=ai-related-none]')).toBeTruthy();
    vi.spyOn(TestBed.inject(AiCitations), 'query').mockResolvedValue('bad query');
    vi.spyOn(TestBed.inject(AiCitations), 'web').mockResolvedValue(null);
    el<HTMLButtonElement>(f, 'button.cite.q').click();
    await settle();
    f.detectChanges();
    expect(el(f, '.err').textContent).toContain('bad query');
  });
});

describe('AiNoVisionCard', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('offers the image support of the active model, downloaded only when asked', async () => {
    const { backend } = setup({ vision: 'missing', stepMs: 5 });
    const install = vi.spyOn(backend, 'installProjector');
    const models = vi.spyOn(TestBed.inject(AiLauncher), 'openModels').mockResolvedValue();
    const f = TestBed.createComponent(AiNoVisionCard);
    f.detectChanges();
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-add-image-support]'));
    expect(install).not.toHaveBeenCalled();
    expect(el(f, '[data-testid=ai-no-vision]').textContent).toContain('Qwen3.5 4B reads text only');
    expect(el(f, '[data-testid=ai-add-image-support]').textContent).toContain('Add image support (640 MB)');
    el<HTMLButtonElement>(f, '[data-testid=ai-choose-vision-model]').click();
    expect(models).toHaveBeenCalled();
    el<HTMLButtonElement>(f, '[data-testid=ai-add-image-support]').click();
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-projector-running]'));
    expect(install).toHaveBeenCalledWith('Qwen3.5-4B-Q4_K_M.gguf');
    expect(el(f, '[data-testid=ai-projector-running]').textContent).toContain('Downloading image support');
    const store = TestBed.inject(AiImageStore);
    await until(f, () => !store.projector().running);
    expect(TestBed.inject(AiStatusStore).status()?.vision).toBe(true);
  });

  it('offers only another model when there is no image support to add, and dismisses', async () => {
    setup({ vision: 'none' });
    const store = TestBed.inject(AiImageStore);
    store.refuseNoVision();
    const f = TestBed.createComponent(AiNoVisionCard);
    f.detectChanges();
    await until(f, () => !!store.activeModel());
    expect(f.nativeElement.querySelector('[data-testid=ai-add-image-support]')).toBeNull();
    expect(el(f, '[data-testid=ai-choose-vision-model]')).toBeTruthy();
    buttonByText(f, 'Dismiss').click();
    expect(store.noVision()).toBe(false);
  });

  it('shows a failed download with Retry, and the generic message without a model', async () => {
    setup({ vision: 'missing', net: 'offline' });
    const store = TestBed.inject(AiImageStore);
    const f = TestBed.createComponent(AiNoVisionCard);
    f.detectChanges();
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-add-image-support]'));
    el<HTMLButtonElement>(f, '[data-testid=ai-add-image-support]').click();
    await until(f, () => !!store.projector().error);
    f.detectChanges();
    expect(el(f, '.err').textContent).toContain('Retry');
    const again = vi.spyOn(store, 'installProjector').mockResolvedValue();
    buttonByText(f, 'Retry').click();
    expect(again).toHaveBeenCalled();
    TestBed.resetTestingModule();
    setup({ mode: 'setup' });
    const g = TestBed.createComponent(AiNoVisionCard);
    g.detectChanges();
    await settle(g);
    expect(el(g, '[data-testid=ai-no-vision]').textContent).toContain('The active model reads text only');
  });
});

describe('AiImagePanel', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  async function panel(options: MockAiOptions = {}, pick: string | null = '/shots/dialog.png') {
    const env = setup(options, pick);
    await TestBed.inject(AiStatusStore).refresh();
    const f = TestBed.createComponent(AiImagePanel);
    f.detectChanges();
    return { ...env, f, store: TestBed.inject(AiImageStore), root: f.nativeElement as HTMLElement };
  }

  it('opens an image, explains it and finds its related logs', async () => {
    const { f, store, root } = await panel({ stepMs: 2 });
    expect(root.textContent).toContain('Open a screenshot');
    el<HTMLButtonElement>(f, '[data-testid=ai-image-open]').click();
    await until(f, () => !!root.querySelector('imt-image-viewer'));
    el<HTMLButtonElement>(f, '[data-testid=ai-image-explain]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-image-explanation] [role=status]'));
    expect(el(f, '[data-testid=ai-image-explanation]').textContent).toMatch(/reads the image|Queued/);
    await until(f, () => !!root.querySelector('[data-testid=ai-image-explanation] imt-ai-rich-text'));
    expect(el(f, '[data-testid=ai-image-explanation]').textContent).toContain('error dialog');
    el<HTMLButtonElement>(f, '[data-testid=ai-image-related]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-related-link]'));
    expect(store.links().length).toBeGreaterThan(0);
    // Two images: a picker switches between them.
    await store.openPath('/shots/second.png');
    f.detectChanges();
    const pick = el<HTMLSelectElement>(f, '[data-testid=ai-image-pick]');
    pick.value = String(store.images()[0]!.id);
    pick.dispatchEvent(new Event('change'));
    f.detectChanges();
    expect(store.current()?.name).toBe('dialog.png');
    expect(root.querySelector('[data-testid=ai-related-link]')).toBeTruthy();
  });

  it('cancels a running request in Jobs, and retries a failed one', async () => {
    const { f, store, backend, root } = await panel({ stepMs: 30 });
    await store.openPath('/shots/dialog.png');
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=ai-image-related]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-related-running] button'));
    const cancel = vi.spyOn(mockJobs(), 'cancel');
    buttonByText(f, 'Cancel').click();
    await until(f, () => cancel.mock.calls.length > 0);
    await until(f, () => !store.currentRelated()?.running);
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new Error('the queue is full'));
    el<HTMLButtonElement>(f, '[data-testid=ai-image-related]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-related-error]'));
    expect(el(f, '[data-testid=ai-related-error]').textContent).toContain('the queue is full');
    buttonByText(f, 'Retry').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-related-link]'));
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new Error('explain failed'));
    el<HTMLButtonElement>(f, '[data-testid=ai-image-explain]').click();
    await until(f, () => !!store.currentExplain()?.error);
    f.detectChanges();
    const retry = vi.spyOn(store, 'explain');
    [...root.querySelectorAll<HTMLButtonElement>('[data-testid=ai-image-explanation] button')].find((b) => b.textContent?.includes('Retry'))!.click();
    expect(retry).toHaveBeenCalled();
  });

  it('shows an open error with Retry, asks in the chat and closes the image', async () => {
    const { f, store, root, ctx } = await panel({}, '/shots/notes.txt');
    el<HTMLButtonElement>(f, '[data-testid=ai-image-open]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-image-open-error]'));
    const retry = vi.spyOn(store, 'retryOpen');
    buttonByText(f, 'Retry').click();
    expect(retry).toHaveBeenCalled();
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-image-open-error]')).toBeNull();
    await store.openPath('/shots/dialog.png');
    f.detectChanges();
    buttonByText(f, 'Ask in chat').click();
    expect(TestBed.inject(ChatStore).attachments().map((a) => a.name)).toEqual(['dialog.png']);
    expect(ctx.last('showView')).toEqual(['ai-chat']);
    el<HTMLButtonElement>(f, 'button[aria-label="Close dialog.png"]').click();
    await until(f, () => !store.images().length);
    expect(root.textContent).toContain('Open a screenshot');
  });

  it('shows a Jobs result whose image was closed, and the no-vision offer of a text-only model', async () => {
    const { f, store, root } = await panel({ vision: 'missing' });
    expect(root.textContent).toContain('The active model reads text only');
    store.showOutcome(9, mockRelatedOutcome('dialog.png'));
    f.detectChanges();
    expect(el(f, '[data-testid=ai-image-detached]').textContent).toContain('The image is no longer open');
    expect(root.querySelector('[data-testid=ai-related-link]')).toBeTruthy();
    await store.openPath('/shots/dialog.png');
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=ai-image-explain]').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-no-vision]'));
  });

  it('names the closed image of a detached result when it knows it', async () => {
    const { f, store } = await panel();
    const img = (await store.openPath('/shots/dialog.png'))!;
    await store.findRelated(img.id);
    const r = store.currentRelated()!;
    await store.close(img.id);
    store.showOutcome(r.jobId!, r.value);
    f.detectChanges();
    expect(el(f, '[data-testid=ai-image-detached]').textContent).toContain('dialog.png is no longer open');
  });
});
