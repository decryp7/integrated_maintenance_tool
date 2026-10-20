import { resolveTokens } from '../../theme/theme-tokens';
import { BUILT_IN_THEMES } from '../../theme/themes';
import { SCHEMES } from '../../theme/theme.types';
import { paletteProblems } from './cvd';

/**
 * Gate on every theme's categorical series tokens (`--s1`…`--s4`): in both schemes every pair separates under
 * protanopia, deuteranopia and tritanopia and under normal vision, and every colour has at least 3:1 contrast against
 * each chart surface (`--bg-1`…`--bg-3`). Charts draw any slot next to any other (small multiples, legends, timeline
 * lanes), so all pairs count, not only neighbours.
 */
describe('series palette tokens', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const scheme of SCHEMES) {
      it(`pass the all-pairs colour-vision and contrast check (${theme.id} ${scheme})`, () => {
        const t = resolveTokens(theme, scheme, 'comfortable');
        const pick = (names: string[]) => names.map((n) => t[n] ?? `missing --${n}`);
        const series = pick(['s1', 's2', 's3', 's4']);
        expect(series.every((c) => /^#[0-9a-f]{6}$/.test(c))).toBe(true);
        expect(paletteProblems(series, pick(['bg-1', 'bg-2', 'bg-3']))).toEqual([]);
      });
    }
  }
});
