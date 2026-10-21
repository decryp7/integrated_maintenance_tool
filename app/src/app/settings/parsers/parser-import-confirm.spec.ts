import { TestBed } from '@angular/core/testing';
import { all, el, settle, text } from '../../../testing/helpers';
import { ParserImportCheckDto } from '../../core/api.types';
import { DIALOG_REF } from '../../ui/dialog.service';
import { lineDiff } from './line-diff';
import { ParserImportConfirm } from './parser-import-confirm';

describe('lineDiff', () => {
  it('keeps common lines and lists removed lines before added ones', () => {
    expect(lineDiff('a\nb\nc\n', 'a\nB\nc\nd')).toEqual([
      { kind: 'same', text: 'a', left: 1, right: 1 },
      { kind: 'del', text: 'b', left: 2, right: null },
      { kind: 'add', text: 'B', left: null, right: 2 },
      { kind: 'same', text: 'c', left: 3, right: 3 },
      { kind: 'add', text: 'd', left: null, right: 4 },
    ]);
    expect(lineDiff('x\r\ny', 'x\ny').every((l) => l.kind === 'same')).toBe(true);
    expect(lineDiff('', 'a')).toEqual([
      { kind: 'del', text: '', left: 1, right: null },
      { kind: 'add', text: 'a', left: null, right: 1 },
    ]);
  });

  it('falls back to removed-then-added for very long texts', () => {
    const big = Array.from({ length: 700 }, (_, i) => `l${i}`).join('\n');
    const diff = lineDiff(big, big.replace('l0', 'L0'));
    expect(diff).toHaveLength(1400);
    expect(diff[0]).toMatchObject({ kind: 'del', left: 1 });
    expect(diff[700]).toMatchObject({ kind: 'add', right: 1 });
  });
});

describe('ParserImportConfirm', () => {
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  const check = (patch: Partial<ParserImportCheckDto>): ParserImportCheckDto => ({
    toml: 'id = "x"\nversion = "1.1.0"\n',
    id: 'x',
    name: 'X logs',
    version: '1.1.0',
    relation: 'newer',
    existing: { origin: 'user', version: '1.0.0', toml: 'id = "x"\nversion = "1.0.0"\n' },
    ...patch,
  });

  async function mount(c: ParserImportCheckDto) {
    ref.close.mockClear();
    TestBed.configureTestingModule({ providers: [{ provide: DIALOG_REF, useValue: ref }] });
    const fixture = TestBed.createComponent(ParserImportConfirm);
    fixture.componentRef.setInput('check', c);
    await settle(fixture);
    return fixture;
  }

  it('shows the versions and the changes of an update', async () => {
    const fixture = await mount(check({}));
    expect(el(fixture, '[data-testid=parser-import-question]').textContent).toContain('Update X logs from version 1.0.0 to 1.1.0?');
    const lines = all(fixture, '[data-testid=parser-import-diff] .line');
    expect(lines.map((l) => l.className.replace('line', '').trim())).toEqual(['same', 'del', 'add']);
    expect(lines[1]?.textContent).toContain('Removed: version = "1.0.0"');
    expect(lines[2]?.textContent).toContain('+');
    expect(all(fixture, '[role=note]')).toHaveLength(0);
    el<HTMLButtonElement>(fixture, '[data-testid=parser-import-replace]').click();
    expect(ref.close).toHaveBeenCalledWith(true);
    expect(el(fixture, '[data-testid=parser-import-replace]').textContent?.trim()).toBe('Update');
  });

  it('asks about the same and an older version, and notes an override of a built-in', async () => {
    let fixture = await mount(check({ relation: 'same', toml: 'id = "x"\nversion = "1.0.0"\n', version: '1.0.0', existing: { origin: 'builtin', version: '1.0.0', toml: 'id = "x"\nversion = "1.0.0"\n' } }));
    expect(text(fixture)).toContain('X logs 1.0.0 is in the library already. Replace it with this file?');
    expect(text(fixture)).toContain('The files are the same.');
    expect(el(fixture, '[role=note]').textContent).toContain('x is built in');
    expect(el(fixture, '[data-testid=parser-import-replace]').textContent?.trim()).toBe('Replace');
    TestBed.resetTestingModule();
    fixture = await mount(check({ relation: 'older', version: '0.9.0' }));
    expect(text(fixture)).toContain('which is newer than this file\'s 0.9.0');
    expect(el(fixture, '[data-testid=parser-import-replace]').classList).toContain('danger');
    (fixture.nativeElement as HTMLElement).querySelector<HTMLButtonElement>('.btn.ghost')!.click();
    expect(ref.close).toHaveBeenCalledWith();
  });
});
