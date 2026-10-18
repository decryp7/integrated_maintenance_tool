import { mockExplainRegex, mockInferRegex } from './mock-regex';
import { at } from '../../../testing/must';

describe('mock regex assistance', () => {
  it('infers a pattern with typed groups, examples and per-line matches', () => {
    const r = mockInferRegex(['took 15 ms', 'took 2 ms', 'other']);
    expect(r.pattern).toBe('took (?P<n1>\\d+) ms');
    expect(r.groups).toEqual([{ name: 'n1', kind: 'number', examples: ['15', '2'] }]);
    expect(r.matches).toEqual([true, true, false]);
    expect(r.explanation.length).toBeGreaterThan(0);
    expect(at(mockInferRegex(['x 2.5 y']).groups, 0).kind).toBe('decimal');
  });

  it('escapes literal text and matches lines without groups', () => {
    const r = mockInferRegex(['(a)', '(a)', 'b']);
    expect(r.pattern).toBe('\\(a\\)');
    expect(r.groups).toEqual([]);
    expect(r.matches).toEqual([true, true, false]);
  });

  it('rejects an empty input like the engine', () => {
    expect(() => mockInferRegex([])).toThrow('at least one');
  });

  it('explains tokens with UTF-8 byte offsets', () => {
    const nodes = mockExplainRegex('^é(?P<n>\\d+)\\.[a-z]+|.+(x)$\\d');
    expect(nodes.map((n) => n.text).slice(0, 13)).toEqual([
      'Start of text',
      'Literal "é"',
      'Start group "n"',
      'One or more digits',
      'End group',
      'Escaped "."',
      'One of the characters [a-z]+',
      'Or',
      'Any characters',
      'Start group',
      'Literal "x"',
      'End group',
      'End of text',
    ]);
    expect(at(nodes, 2).start).toBe(3); // 'é' is two bytes
    expect(nodes.every((n) => n.depth === 0)).toBe(true);
  });
});
