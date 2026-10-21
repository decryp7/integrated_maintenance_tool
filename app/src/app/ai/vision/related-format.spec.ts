import { describe, expect, it } from 'vitest';
import { AiImage, RelatedLink, RelatedOutcome } from '../ai.types';
import { bytesToBase64, dragHasFiles, imageFilesOf, imageFilter, isImageFile, isImagePath, MAX_IMAGES_PER_QUESTION } from './image-files';
import {
  capturedLabel,
  groupBySource,
  imageLinks,
  imageTimeLabel,
  noteLabel,
  parseNarrative,
  parseRelatedOutcome,
  reasonLabel,
  screenKindLabel,
  sizeLabel,
} from './related-format';

const time = (ms: number | null) => (ms === null ? '—' : `t${ms}`);

function link(id: number, sourceName: string): RelatedLink {
  return {
    event: { id, ts: id * 1000, level: 'Error', message: `m${id}`, exception: null, source: 0, sourceName, group: '', fields: {} },
    score: 10 - id,
    reasons: [{ kind: 'time', deltaMs: 1400 }],
  };
}

function outcome(links: RelatedLink[] = [link(1, 'a.log')]): RelatedOutcome {
  return {
    facts: { screenKind: 'error_dialog', windowTitle: null, application: null, timestamps: [], errorTexts: [], errorCodes: [], exceptionNames: [], machineNames: [], userNames: [], serverNames: [], ids: [], versions: [], summary: '' },
    result: { time: null, links, notes: [], examined: 3 },
    explanation: { text: 'x', citations: [], evidence: { recordIds: [], queries: [] }, usage: { inputTokens: 0, outputTokens: 0 } },
  };
}

const image: AiImage = { id: 1, name: 'a.png', path: null, width: 100, height: 50, originalWidth: 100, originalHeight: 50, sourceFormat: 'png', dataUrl: 'data:image/png;base64,AA', capturedAt: null };

describe('related-format', () => {
  it('puts each reason into words', () => {
    expect(reasonLabel({ kind: 'time', deltaMs: 1400 })).toBe('time +1.4 s');
    expect(reasonLabel({ kind: 'time', deltaMs: -61_000 })).toBe('time -1 min 1 s');
    expect(reasonLabel({ kind: 'title', text: 'x', target: { kind: 'lane', lane: 'OrderWorker' } })).toBe('title matches lane OrderWorker');
    expect(reasonLabel({ kind: 'title', text: 'x', target: { kind: 'source', name: 'web.log' } })).toBe('title matches source web.log');
    expect(reasonLabel({ kind: 'title', text: 'x', target: { kind: 'field', field: 'Application', value: 'Shop' } })).toBe('title matches Application Shop');
    expect(reasonLabel({ kind: 'text', clue: 'HttpRequestException', clueKind: 'exceptionName', partial: false })).toBe("text 'HttpRequestException'");
    expect(reasonLabel({ kind: 'text', clue: 'disk full', clueKind: 'errorText', partial: true })).toBe("text close to 'disk full'");
  });

  it('puts each note into words', () => {
    expect(noteLabel({ kind: 'noTime' }, time)).toContain('matched by text and title only');
    expect(noteLabel({ kind: 'outsideData', from: 1, to: null }, time)).toBe("The image's time is outside the loaded data (t1 – —).");
    expect(noteLabel({ kind: 'clockDisagreement', deltaMs: -180_000, sourceId: 2, sourceName: 'web02.log' }, time)).toBe(
      "The clocks likely disagree: the best match in web02.log is -3 min away from the image's time.",
    );
    expect(noteLabel({ kind: 'offsetApplied', sourceName: 'web02.log', offsetMs: 2000 }, time)).toBe('Applied the clock offset of web02.log (+2 s).');
    expect(noteLabel({ kind: 'nothingToSearch' }, time)).toContain('nothing on the image');
  });

  it('says where the time came from and what kind of screen it is', () => {
    const at = (source: Parameters<typeof imageTimeLabel>[0]['source']) => imageTimeLabel({ atMs: 5, precisionMs: 1000, source, text: '' }, time);
    expect(at('screen')).toBe('Taken at t5 (shown on the screen)');
    expect(at('screenTimeOfDay')).toContain('time of day');
    expect(at('exif')).toContain('capture time');
    expect(at('fileName')).toContain('file name');
    expect(at('fileModified')).toContain('modification time');
    expect(screenKindLabel('error_dialog')).toBe('Error dialog');
    for (const k of ['frozen_window', 'console', 'dashboard', 'log_viewer', 'browser', 'other'] as const) expect(screenKindLabel(k)).toBeTruthy();
    expect(screenKindLabel('weird' as never)).toBe('Screenshot');
  });

  it('describes the clean copy against the original', () => {
    expect(sizeLabel(image)).toBe('100 × 50 · PNG');
    expect(sizeLabel({ ...image, originalWidth: 200, originalHeight: 100, sourceFormat: 'jpeg' })).toBe('100 × 50 · original 200 × 100 JPEG');
    expect(capturedLabel(image)).toBeNull();
    expect(capturedLabel({ ...image, capturedAt: '2026-10-14 10:41:26' })).toBe('Captured 2026-10-14 10:41:26');
  });

  it('groups links by source keeping the engine order, and turns them into image links', () => {
    const ls = [link(1, 'a.log'), link(2, 'b.log'), link(3, 'a.log')];
    expect(groupBySource(ls).map((g) => [g.sourceName, g.links.map((l) => l.event.id)])).toEqual([
      ['a.log', [1, 3]],
      ['b.log', [2]],
    ]);
    expect(imageLinks(7, 'a.png', outcome(ls))).toEqual([
      { imageId: 7, imageName: 'a.png', eventId: 1, reasons: ls[0]!.reasons },
      { imageId: 7, imageName: 'a.png', eventId: 2, reasons: ls[1]!.reasons },
      { imageId: 7, imageName: 'a.png', eventId: 3, reasons: ls[2]!.reasons },
    ]);
  });

  it('parses job results without trusting them', () => {
    const o = outcome();
    expect(parseRelatedOutcome(o)).toBe(o);
    expect(parseRelatedOutcome(null)).toBeNull();
    expect(parseRelatedOutcome({ facts: {}, result: { links: [] } })).toBeNull();
    expect(parseRelatedOutcome({ facts: {}, result: { links: [], notes: [] }, explanation: {} })).toBeNull();
    expect(parseRelatedOutcome({ result: {} })).toBeNull();
    expect(parseNarrative(o.explanation)).toBe(o.explanation);
    expect(parseNarrative({ text: 1 })).toBeNull();
    expect(parseNarrative('x')).toBeNull();
  });
});

describe('image-files', () => {
  it('recognises the image formats the engine reads', () => {
    for (const p of ['a.png', 'B.JPG', 'c.jpeg', 'd.bmp', 'e.gif', 'f.webp', 'C:\\shots\\g.PNG']) expect(isImagePath(p)).toBe(true);
    for (const p of ['a.log', 'png', 'a.svg', 'a.tiff', '']) expect(isImagePath(p)).toBe(false);
    expect(isImageFile({ type: 'image/png', name: 'x' })).toBe(true);
    expect(isImageFile({ type: 'image/svg+xml', name: 'x.png' })).toBe(false);
    expect(isImageFile({ type: '', name: 'x.webp' })).toBe(true);
    expect(MAX_IMAGES_PER_QUESTION).toBe(4);
    expect(imageFilter()[0]?.extensions).toEqual(['png', 'jpg', 'jpeg', 'bmp', 'gif', 'webp']);
  });

  it('takes the image files of a paste or a drop', () => {
    const png = new File([new Uint8Array([1])], 'a.png', { type: 'image/png' });
    const txt = new File(['x'], 'a.txt', { type: 'text/plain' });
    expect(imageFilesOf({ files: [png, txt] as unknown as FileList, items: [] as unknown as DataTransferItemList })).toEqual([png]);
    const items = [
      { kind: 'string', getAsFile: () => null },
      { kind: 'file', getAsFile: () => png },
      { kind: 'file', getAsFile: () => null },
    ];
    expect(imageFilesOf({ files: [] as unknown as FileList, items: items as unknown as DataTransferItemList })).toEqual([png]);
    expect(imageFilesOf(null)).toEqual([]);
    expect(dragHasFiles({ types: ['Files'] })).toBe(true);
    expect(dragHasFiles({ types: ['text/plain'] })).toBe(false);
    expect(dragHasFiles(null)).toBe(false);
  });

  it('encodes bytes as base64 in chunks', () => {
    expect(bytesToBase64(new Uint8Array([]))).toBe('');
    expect(bytesToBase64(new Uint8Array([0x89, 0x50, 0x4e, 0x47]))).toBe('iVBORw==');
    const big = new Uint8Array(100_000).map((_, i) => i % 256);
    const decoded = atob(bytesToBase64(big));
    expect(decoded.length).toBe(big.length);
    expect(decoded.charCodeAt(70_000)).toBe(70_000 % 256);
  });
});
