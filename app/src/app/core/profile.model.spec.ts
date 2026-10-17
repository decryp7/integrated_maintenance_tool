import { toJsRegex } from './profile.model';

describe('toJsRegex', () => {
  it('converts Rust named groups', () => {
    const m = toJsRegex('CPU (?P<cpu>\\d+)').exec('CPU 42');
    expect(m?.groups?.['cpu']).toBe('42');
  });

  it('lifts leading inline flags', () => {
    const re = toJsRegex('(?is)start.end', 'd');
    expect(re.flags).toBe('dis');
    expect(re.test('START\nEND')).toBe(true);
  });

  it('ignores unsupported inline flags and does not duplicate', () => {
    expect(toJsRegex('(?ix)a', 'i').flags).toBe('i');
    expect(toJsRegex('a').flags).toBe('');
  });
});
