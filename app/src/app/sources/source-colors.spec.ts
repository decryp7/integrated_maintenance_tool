import { resolveTokens } from '../theme/theme-tokens';
import { BUILT_IN_THEMES } from '../theme/themes';
import { SCHEMES } from '../theme/theme.types';
import { CVD_KINDS, deltaE, paletteProblems, worstCvdDelta } from '../tools/monitor/cvd';
import { InstanceSlots, SOURCE_COLOR_VARS, stripeBackground, swatchAt } from './source-colors';

/**
 * Gate on every theme's source-marker tokens (`--src-1`…`--src-4`): in both schemes every pair separates under
 * protanopia, deuteranopia and tritanopia (Machado 2009, OKLab ΔE ≥ 8) and normal vision (ΔE ≥ 15), and every colour
 * has non-text contrast ≥ 3:1 against every surface a marker is drawn on: the panels and the grid rows, hovered,
 * active and selected (WCAG 1.4.11).
 */
const SURFACES = ['bg-1', 'bg-2', 'bg-3', 'bg-hover', 'bg-active', 'bg-sel'];

describe('source marker palette', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const scheme of SCHEMES) {
      it(`passes the all-pairs colour-vision and 3:1 contrast gate (${theme.id} ${scheme})`, () => {
        const tokens = resolveTokens(theme, scheme, 'comfortable');
        const pick = (names: string[]) => names.map((n) => tokens[n] ?? `missing --${n}`);
        const colors = pick(SOURCE_COLOR_VARS.map((_, i) => `src-${i + 1}`));
        expect(colors).toHaveLength(4);
        expect(paletteProblems(colors, pick(SURFACES))).toEqual([]);
        colors.forEach((a, i) =>
          colors.slice(i + 1).forEach((b) => {
            for (const kind of CVD_KINDS) expect(deltaE(a, b, kind)).toBeGreaterThanOrEqual(8);
            expect(worstCvdDelta(a, b, CVD_KINDS)).toBeGreaterThanOrEqual(8);
            expect(deltaE(a, b)).toBeGreaterThanOrEqual(15);
          }),
        );
      });
    }
  }

  it('the check catches a confusable pair and a low-contrast colour', () => {
    expect(paletteProblems(['#1795fa', '#1795fb'], ['#0f1117']).length).toBeGreaterThan(0);
    expect(paletteProblems(['#20232b'], ['#0f1117'])[0]).toMatch(/contrast/);
  });
});

describe('swatches', () => {
  it('cycle colours first, then patterns, by slot', () => {
    expect([0, 1, 2, 3].map((i) => swatchAt(i).color)).toEqual([...SOURCE_COLOR_VARS]);
    expect(swatchAt(0).pattern).toBe('solid');
    expect(swatchAt(4)).toEqual({ slot: 4, color: SOURCE_COLOR_VARS[0], pattern: 'dashed' });
    expect(swatchAt(9)).toEqual({ slot: 9, color: SOURCE_COLOR_VARS[1], pattern: 'dotted' });
    expect(swatchAt(12).pattern).toBe('solid');
    // Twelve slots are pairwise distinct (colour or pattern).
    const combos = new Set(Array.from({ length: 12 }, (_, i) => `${swatchAt(i).color}|${swatchAt(i).pattern}`));
    expect(combos.size).toBe(12);
    expect(swatchAt(-3).slot).toBe(0);
    expect(swatchAt(Number.NaN).slot).toBe(0);
  });

  it('draw solid, dashed and dotted stripes', () => {
    expect(stripeBackground(swatchAt(0))).toBe('var(--src-1)');
    expect(stripeBackground(swatchAt(4))).toContain('6px 9px');
    expect(stripeBackground(swatchAt(8))).toContain('3px 6px');
  });

  it('keep an instance slot for good, whatever appears later', () => {
    const slots = new InstanceSlots();
    expect([slots.slotOf('web01'), slots.slotOf('web02'), slots.slotOf('web01')]).toEqual([0, 1, 0]);
    expect(slots.slotOf('worker')).toBe(2);
    expect(slots.slotOf('web02')).toBe(1);
  });
});
