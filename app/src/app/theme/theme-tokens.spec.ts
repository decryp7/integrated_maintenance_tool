import { effectiveTraits, missingTokens, px, resolveTokens, SYSTEM_FONT_SANS, traitAttribute, traitAttributes, withUiFont } from './theme-tokens';
import { BUILT_IN_THEMES, DEFAULT_THEME_ID, isThemeId, themeById } from './themes';
import { CALM } from './themes/calm.theme';
import { CONSOLE } from './themes/console.theme';
import { PRECISION } from './themes/precision.theme';
import { THEME_IDS, TRAIT_VALUES, TraitName } from './theme.types';

describe('theme tokens', () => {
  it('resolve common, then scheme colours, then density sizes', () => {
    const dark = resolveTokens(CALM, 'dark', 'comfortable');
    expect(dark['font-sans']).toContain('Inter');
    expect(dark['bg-1']).toBe('#17181b');
    expect(dark['row-h']).toBe('24px');
    const light = resolveTokens(CALM, 'light', 'compact');
    expect(light['bg-1']).toBe('#ffffff');
    expect(light['row-h']).toBe('22px');
    expect(resolveTokens(CONSOLE, 'dark', 'comfortable')['font-sans']).toContain('Geist Sans');
  });

  it('report what an incomplete theme is missing, per scheme and density', () => {
    const broken = { ...CALM, common: { ...CALM.common, 'fs-md': '' }, schemes: { ...CALM.schemes, light: { text: '#000000' } }, densities: { ...CALM.densities, compact: {} } };
    const missing = missingTokens(broken);
    expect(missing).toContain('common: --fs-md');
    expect(missing).toContain('light: --bg-1');
    expect(missing).toContain('compact: --row-h');
    expect(missing.some((m) => m.startsWith('dark:'))).toBe(false);
  });

  it('apply the user override for lane colouring, and only that', () => {
    expect(effectiveTraits(CALM, { lanesBySource: null })).toBe(CALM.traits);
    expect(effectiveTraits(CALM, { lanesBySource: true })).toEqual({ ...CALM.traits, laneFill: 'bySource' });
    expect(effectiveTraits(CONSOLE, { lanesBySource: false })).toEqual({ ...CONSOLE.traits, laneFill: 'neutral' });
  });

  it('name trait attributes in kebab case, every trait once', () => {
    expect(traitAttribute('laneFill')).toBe('data-lane-fill');
    expect(traitAttribute('runs')).toBe('data-runs');
    const attrs = traitAttributes(CONSOLE.traits);
    expect(attrs.map(([a]) => a)).toEqual(['data-surface', 'data-runs', 'data-lane-fill', 'data-crash-band', 'data-brush-pill', 'data-findings', 'data-numerals']);
    expect(Object.fromEntries(attrs)['data-runs']).toBe('pill');
  });

  it('use the operating system font for the interface unless the user keeps the theme font; mono stays', () => {
    const tokens = resolveTokens(PRECISION, 'light', 'compact');
    const system = withUiFont(tokens, 'system');
    expect(system['font-sans']).toBe(SYSTEM_FONT_SANS);
    expect(system['font-mono']).toBe(tokens['font-mono']);
    expect(withUiFont(tokens, 'theme')).toBe(tokens);
    // Windows, macOS and Linux desktop faces, then the generic family.
    for (const face of ["'Segoe UI Variable Text'", "'Segoe UI'", '-apple-system', 'system-ui', 'Cantarell', 'Ubuntu']) expect(SYSTEM_FONT_SANS).toContain(face);
    expect(SYSTEM_FONT_SANS.endsWith('sans-serif')).toBe(true);
  });

  it('read pixel tokens', () => {
    expect(px('24px')).toBe(24);
    expect(px(' 10.5px ')).toBe(10.5);
    expect(px('1rem')).toBe(0);
    expect(px(undefined)).toBe(0);
  });
});

describe('built-in theme registry', () => {
  it('has one theme per id, the default first, each with valid traits', () => {
    expect(BUILT_IN_THEMES.map((t) => t.id)).toEqual([...THEME_IDS]);
    expect(BUILT_IN_THEMES[0]?.id).toBe(DEFAULT_THEME_ID);
    for (const t of BUILT_IN_THEMES) {
      for (const [name, values] of Object.entries(TRAIT_VALUES)) expect(values as readonly string[]).toContain(t.traits[name as TraitName]);
    }
  });

  it('narrows ids and falls back to the default for unknown ones', () => {
    expect(isThemeId('console')).toBe(true);
    expect(isThemeId('precision')).toBe(true);
    expect(isThemeId('neon')).toBe(false);
    expect(isThemeId(3)).toBe(false);
    expect(themeById('console')).toBe(CONSOLE);
    expect(themeById('precision')).toBe(PRECISION);
    expect(themeById('neon')).toBe(CALM);
  });

  it('gives the themes different looks, not only different colours', () => {
    expect(CALM.traits.runs).not.toBe(CONSOLE.traits.runs);
    expect(CALM.traits.surface).not.toBe(CONSOLE.traits.surface);
    expect(CALM.common['font-sans']).not.toBe(CONSOLE.common['font-sans']);
    expect(PRECISION.traits.runs).toBe('bracket');
    expect(PRECISION.traits.laneFill).toBe('barcode');
    expect(PRECISION.common['font-sans']).toContain('IBM Plex Sans');
    expect(PRECISION.common['font-mono']).toContain('IBM Plex Mono');
  });
});
