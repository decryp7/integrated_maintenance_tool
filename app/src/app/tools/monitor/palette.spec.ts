import { readFileSync } from 'node:fs';
import { paletteProblems } from './cvd';

/**
 * Gate on the theme's categorical series tokens (`--s1`…`--s4` in styles.scss): in both themes every pair separates
 * under protanopia, deuteranopia and tritanopia and under normal vision, and every colour has at least 3:1 contrast
 * against each chart surface (`--bg-1`…`--bg-3`). Charts draw any slot next to any other (small multiples, legends,
 * timeline lanes), so all pairs count, not only neighbours.
 */

const STYLES = readFileSync('src/styles.scss', 'utf8');

/** `--name: #rrggbb` declarations of one block of the stylesheet. */
function tokens(block: string): Map<string, string> {
  return new Map([...block.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map((m) => [m[1] ?? '', (m[2] ?? '').toLowerCase()]));
}

function theme(block: string): { series: string[]; surfaces: string[] } {
  const t = tokens(block);
  const pick = (names: string[]) => names.map((n) => t.get(n) ?? `missing --${n}`);
  return { series: pick(['s1', 's2', 's3', 's4']), surfaces: pick(['bg-1', 'bg-2', 'bg-3']) };
}

const lightStart = STYLES.indexOf('@mixin light-tokens');
const DARK = theme(STYLES.slice(STYLES.indexOf(':root {'), lightStart));
const LIGHT = theme(STYLES.slice(lightStart, STYLES.indexOf('@media (prefers-color-scheme: light)')));

describe('series palette tokens', () => {
  it('are read from the stylesheet', () => {
    for (const t of [DARK, LIGHT]) {
      expect(t.series.every((c) => /^#[0-9a-f]{6}$/.test(c))).toBe(true);
      expect(t.surfaces.every((c) => /^#[0-9a-f]{6}$/.test(c))).toBe(true);
    }
    expect(DARK.series).not.toEqual(LIGHT.series);
  });

  it('pass the all-pairs colour-vision and contrast check in the dark theme', () => {
    expect(paletteProblems(DARK.series, DARK.surfaces)).toEqual([]);
  });

  it('pass the all-pairs colour-vision and contrast check in the light theme', () => {
    expect(paletteProblems(LIGHT.series, LIGHT.surfaces)).toEqual([]);
  });
});
