import { didYouMean, editDistance, suggestionScore, tolerance } from './did-you-mean';

describe('did you mean', () => {
  it('measures edit distance case-insensitively, counting adjacent swaps once', () => {
    expect(editDistance('Export', 'export')).toBe(0);
    expect(editDistance('', 'abc')).toBe(3);
    expect(editDistance('abc', '')).toBe(3);
    expect(editDistance('exprot', 'export')).toBe(1);
    expect(editDistance('kitten', 'sitting')).toBe(3);
    expect(editDistance('ab', 'ba')).toBe(1);
  });

  it('tolerates one typo per three characters, at least one', () => {
    expect(tolerance('ab')).toBe(1);
    expect(tolerance('timeline')).toBe(2);
  });

  it('scores every query word against its closest label word, prefixes counting as exact', () => {
    expect(suggestionScore('exprot', 'File Export events')).toBe(1);
    expect(suggestionScore('exp', 'Export events')).toBe(0);
    expect(suggestionScore('zzzz', 'Export events')).toBeNull();
    expect(suggestionScore('', 'Export')).toBeNull();
    expect(suggestionScore('x', '…')).toBeNull();
  });

  it('ranks items best first, keeps ties in order and limits the list', () => {
    const items = ['Toggle timeline panel', 'Toggle sidebar', 'Show timeline', 'Open folder'];
    expect(didYouMean(items, 'timelnie', (x) => x)).toEqual(['Toggle timeline panel', 'Show timeline']);
    expect(didYouMean(items, 'toggel', (x) => x, 1)).toEqual(['Toggle timeline panel']);
    expect(didYouMean(items, 'qqqqqq', (x) => x)).toEqual([]);
  });
});
