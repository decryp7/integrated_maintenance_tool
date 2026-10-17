import { TestBed } from '@angular/core/testing';
import { IconFilePicker, MAX_ICON_BYTES, readIconFile } from './icon-import';

const file = (name: string, content: BlobPart, type = '') => new File([content], name, { type });

describe('readIconFile', () => {
  it('reads svg text and png bytes', async () => {
    const svg = await readIconFile(file('Star Logo.svg', '<svg viewBox="0 0 1 1"/>', 'image/svg+xml'), new Set());
    expect(svg).toEqual({ ok: true, icon: { name: 'Star-Logo', svg: '<svg viewBox="0 0 1 1"/>' } });
    const png = await readIconFile(file('a.png', new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2])), new Set(['a']));
    expect(png).toMatchObject({ ok: true, icon: { name: 'a-2', png: btoa('\x89PNG\x01\x02'), pngDataUrl: 'data:image/png;base64,' + btoa('\x89PNG\x01\x02') } });
  });
  it('rejects oversized, mislabelled and unsupported files', async () => {
    expect(await readIconFile(file('big.svg', 'x'.repeat(MAX_ICON_BYTES + 1)), new Set())).toMatchObject({ ok: false, error: expect.stringContaining('16 KB') });
    expect(await readIconFile(file('a.png', 'not a png'), new Set())).toMatchObject({ ok: false, error: 'That file is not a PNG image.' });
    expect(await readIconFile(file('a.svg', '<p/>'), new Set())).toMatchObject({ ok: false, error: 'That file is not an SVG image.' });
    expect(await readIconFile(file('a.gif', 'GIF'), new Set())).toMatchObject({ ok: false, error: 'Choose an .svg or .png file.' });
    const broken = file('a.svg', 'x');
    vi.spyOn(broken, 'text').mockRejectedValue(new Error('io'));
    expect(await readIconFile(broken, new Set())).toMatchObject({ ok: false, error: 'The file could not be read.' });
  });
});

describe('IconFilePicker', () => {
  const run = (act: (input: HTMLInputElement) => void) => {
    const spy = vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (this: HTMLInputElement) {
      act(this);
    });
    return TestBed.inject(IconFilePicker).pick().finally(() => spy.mockRestore());
  };
  it('resolves with the chosen file or null on cancel', async () => {
    const f = file('a.svg', '<svg/>');
    const chosen = await run((input) => {
      Object.defineProperty(input, 'files', { value: [f] });
      expect(input.accept).toContain('.svg');
      input.dispatchEvent(new Event('change'));
    });
    expect(chosen).toBe(f);
    expect(await run((input) => input.dispatchEvent(new Event('cancel')))).toBeNull();
    expect(await run((input) => input.dispatchEvent(new Event('change')))).toBeNull();
  });
});
