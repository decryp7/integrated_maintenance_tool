import { contrastProblems, paletteProblemsOf } from './theme-checks';
import { missingTokens, resolveTokens } from './theme-tokens';
import { BUILT_IN_THEMES } from './themes';
import { CONTRAST } from './themes/contrast.theme';
import { DENSITIES, SCHEMES } from './theme.types';

/**
 * The quality gates of ADR 0018 over every built-in theme × scheme: complete token sets, AA text contrast on every
 * surface, 3:1 marks, and categorical palettes that separate under colour-vision deficiency.
 */
describe('built-in themes', () => {
  for (const theme of BUILT_IN_THEMES) {
    it(`${theme.id} defines every token for both schemes and densities`, () => {
      expect(missingTokens(theme)).toEqual([]);
    });

    for (const scheme of SCHEMES) {
      const tokens = resolveTokens(theme, scheme, DENSITIES[0]);

      it(`${theme.id} ${scheme}: text and marks meet their contrast minimums`, () => {
        expect(contrastProblems(tokens)).toEqual([]);
      });

      it(`${theme.id} ${scheme}: source and series palettes pass the colour-vision check`, () => {
        expect(paletteProblemsOf(tokens)).toEqual([]);
      });
    }
  }
});

describe('theme checks', () => {
  const ok = resolveTokens(BUILT_IN_THEMES[0]!, 'dark', 'comfortable');

  it('report a low-contrast text pair, a missing token and a colour that is not #rrggbb', () => {
    expect(contrastProblems({ ...ok, muted: ok['bg-1']! })[0]).toMatch(/--muted .* has contrast 1\.\d\d:1, below 4\.5:1/);
    const { faint: _faint, ...without } = ok;
    expect(contrastProblems(without)).toContain('--faint on --bg-0: missing');
    expect(contrastProblems({ ...ok, text: 'red' })[0]).toMatch(/not #rrggbb/);
  });

  it('report a weak mark and confusable source colours', () => {
    expect(contrastProblems({ ...ok, 'mk-error': ok['lane-bar']! }).some((p) => p.startsWith('--mk-error'))).toBe(true);
    expect(paletteProblemsOf({ ...ok, 'src-2': ok['src-1']! }).some((p) => p.startsWith('sources:') && p.includes('confusable'))).toBe(true);
  });
});

describe('high contrast theme', () => {
  for (const scheme of SCHEMES) {
    for (const density of DENSITIES) {
      it(`${scheme}, ${density}: text is AAA (7:1), marks 4.5:1, borders and the focus ring 3:1`, () => {
        expect(contrastProblems(resolveTokens(CONTRAST, scheme, density), 'AAA')).toEqual([]);
      });
    }

    it(`${scheme}: is complete and its palettes pass the colour-vision check`, () => {
      expect(missingTokens(CONTRAST)).toEqual([]);
      expect(paletteProblemsOf(resolveTokens(CONTRAST, scheme, 'comfortable'))).toEqual([]);
    });
  }

  it('is selectable and reports what falls short of AAA', () => {
    expect(BUILT_IN_THEMES.map((t) => t.id)).toContain('contrast');
    const dark = resolveTokens(CONTRAST, 'dark', 'comfortable');
    // Calm passes AA but not AAA: the gate tells them apart.
    const calm = resolveTokens(BUILT_IN_THEMES[0]!, 'dark', 'comfortable');
    expect(contrastProblems(calm, 'AA')).toEqual([]);
    expect(contrastProblems(calm, 'AAA').length).toBeGreaterThan(0);
    expect(contrastProblems({ ...dark, border: dark['bg-1']! }, 'AAA').some((p) => p.startsWith('--border '))).toBe(true);
    expect(contrastProblems({ ...dark, 'focus-ring': dark['bg-1']! }, 'AAA').some((p) => p.startsWith('--focus-ring'))).toBe(true);
  });
});
