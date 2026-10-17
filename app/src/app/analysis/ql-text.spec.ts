import { andQl, applyCompletion, byteToChar, charToByte, errorSpan, hasTopLevelOr, highlight, qlEquals, qlField, qlQuote, wordAt } from './ql-text';

describe('ql-text', () => {
  it('classifies tokens', () => {
    const segs = highlight(`level >= warning and msg contains "x y" | stats count(), p95(ms) as p by source | head 10 \`odd name\` @message 'q' (1.5) -3 ?`);
    const by = (cls: string) => segs.filter((s) => s.cls === cls).map((s) => s.text);
    expect(by('kw')).toEqual(expect.arrayContaining(['and', 'contains', 'stats', 'as', 'by', 'head']));
    expect(by('fn')).toEqual(['count', 'p95']);
    expect(by('op')).toEqual(['>=']);
    expect(by('str')).toEqual(['"x y"', "'q'"]);
    expect(by('field')).toEqual(['`odd name`', '@message']);
    expect(by('num')).toEqual(expect.arrayContaining(['10', '1.5', '-3']));
    expect(by('punct')).toEqual(expect.arrayContaining(['|', '(', ')', ',']));
    expect(segs.map((s) => s.text).join('')).toBe(`level >= warning and msg contains "x y" | stats count(), p95(ms) as p by source | head 10 \`odd name\` @message 'q' (1.5) -3 ?`);
    expect(by('text')).toContain('?');
  });

  it('keeps unterminated strings and non-function names as text/keywords', () => {
    expect(highlight('msg = "abc')[4]).toMatchObject({ cls: 'str', text: '"abc' });
    expect(highlight('count').map((s) => s.cls)).toEqual(['text']);
    expect(highlight('')).toEqual([]);
  });

  it('splits segments at the error span', () => {
    const segs = highlight('level >= oops and x', { message: 'm', offset: 9, len: 4 });
    expect(segs.filter((s) => s.err).map((s) => s.text).join('')).toBe('oops');
    expect(segs.map((s) => s.text).join('')).toBe('level >= oops and x');
    const wide = highlight('a b c', { message: 'm', offset: 0, len: 3 });
    expect(wide.filter((s) => s.err).map((s) => s.text).join('')).toBe('a b');
  });

  it('does not mark zero-length errors', () => {
    expect(highlight('a |', { message: 'm', offset: 3, len: 0 }).some((s) => s.err)).toBe(false);
    expect(errorSpan('a |', { message: 'm', offset: 3, len: 0 })).toEqual({ start: 3, end: 3 });
    expect(errorSpan('x', null)).toBeNull();
  });

  it('converts between UTF-8 byte offsets and string indices', () => {
    const t = 'é = "ü" x';
    expect(charToByte(t, 1)).toBe(2);
    expect(byteToChar(t, 2)).toBe(1);
    expect(byteToChar(t, 999)).toBe(t.length);
    expect(byteToChar(t, -4)).toBe(0);
    expect(errorSpan(t, { message: 'm', offset: 2, len: 2 })).toEqual({ start: 1, end: 3 });
  });

  it('applies a completion over the word being typed', () => {
    expect(applyCompletion('lev', 3, 'level')).toEqual({ text: 'level', cursor: 5 });
    expect(applyCompletion('a = 1 | st', 10, 'stats')).toEqual({ text: 'a = 1 | stats', cursor: 13 });
    expect(applyCompletion('level ', 6, '>=')).toEqual({ text: 'level >=', cursor: 8 });
    expect(applyCompletion('x | stats co', 12, 'count()')).toEqual({ text: 'x | stats count()', cursor: 16 });
    expect(applyCompletion('lev and', 3, 'level')).toEqual({ text: 'level and', cursor: 5 });
    expect(wordAt('foo @bar', 8)).toBe('@bar');
    expect(wordAt('foo ', 4)).toBe('');
  });

  it('quotes fields and values for the engine', () => {
    expect(qlQuote('a"b\\c')).toBe('"a\\"b\\\\c"');
    expect(qlField('UserName')).toBe('UserName');
    expect(qlField('@level')).toBe('@level');
    expect(qlField('user.id')).toBe('user.id');
    expect(qlField('User Name')).toBe('`User Name`');
    expect(qlField('a|b')).toBe('`a|b`');
    expect(qlField('or')).toBe('`or`');
    expect(qlField('we`ird x')).toBe('`weird x`');
    expect(qlEquals('User Name', 'ann "x"')).toBe('`User Name` = "ann \\"x\\""');
  });

  it('finds an or outside parentheses, strings and backticks only', () => {
    expect(hasTopLevelOr('a = 1 or b = 2')).toBe(true);
    expect(hasTopLevelOr('a = 1 OR b = 2')).toBe(true);
    expect(hasTopLevelOr('(a = 1 or b = 2) and c = 3')).toBe(false);
    expect(hasTopLevelOr('msg = "this or that"')).toBe(false);
    expect(hasTopLevelOr('`x or y` = 1')).toBe(false);
    expect(hasTopLevelOr('a = 1) or (b = 2')).toBe(true);
  });

  it('ANDs a clause into the filter part of a query', () => {
    expect(andQl('', 'u = "a"')).toBe('u = "a"');
    expect(andQl('   ', 'u = "a"')).toBe('u = "a"');
    expect(andQl('level >= error', '')).toBe('level >= error');
    expect(andQl('level >= error', 'u = "a"')).toBe('level >= error and u = "a"');
    expect(andQl('a = 1 or b = 2', 'u = "a"')).toBe('(a = 1 or b = 2) and u = "a"');
    expect(andQl('a = 1', 'x = 1 or y = 2')).toBe('a = 1 and (x = 1 or y = 2)');
    expect(andQl('u = "a"', 'u = "a"')).toBe('u = "a"');
    expect(andQl('level >= error | stats count() by Host', 'u = "a"')).toBe('level >= error and u = "a" | stats count() by Host');
    expect(andQl('| head 5', 'u = "a"')).toBe('u = "a" | head 5');
    expect(andQl('msg = "a|b"', 'u = "a"')).toBe('msg = "a|b" and u = "a"');
    expect(andQl('msg = "say \\"|\\"" | head 1', 'u = 1')).toBe('msg = "say \\"|\\"" and u = 1 | head 1');
    expect(andQl('`a|b` = 1', 'u = 1')).toBe('`a|b` = 1 and u = 1');
  });
});
