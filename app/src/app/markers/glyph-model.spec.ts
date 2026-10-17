import { BUILTIN_ICONS } from './builtin-icons';
import { badgeFontSize, badgeShape, badgeText, contrastRatio, inkFor, normalizeHex, resolveGlyph, sanitizeSvg, shapeGeometry } from './glyph-model';

describe('colours', () => {
  it('normalises hex colours', () => {
    expect(normalizeHex('#ABC')).toBe('#aabbcc');
    expect(normalizeHex(' #A1B2C3 ')).toBe('#a1b2c3');
    expect(normalizeHex('red')).toBeNull();
    expect(normalizeHex(null)).toBeNull();
    expect(normalizeHex('#12')).toBeNull();
  });

  it('computes WCAG contrast', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 0);
    expect(contrastRatio('#777777', '#777777')).toBe(1);
  });

  it('picks an ink colour with at least 4.5:1 for every fill', () => {
    let worst = 21;
    for (let r = 0; r < 256; r += 15) for (let g = 0; g < 256; g += 15) for (let b = 0; b < 256; b += 15) {
      const fill = '#' + [r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('');
      worst = Math.min(worst, contrastRatio(fill, inkFor(fill)));
    }
    expect(worst).toBeGreaterThanOrEqual(4.5);
    expect(inkFor('#ffffff')).toBe('#000000');
    expect(inkFor('#000000')).toBe('#ffffff');
  });
});

describe('badges', () => {
  it('limits text to three visible characters', () => {
    expect(badgeText(' DB ')).toBe('DB');
    expect(badgeText('ABCDE')).toBe('ABC');
    expect(badgeText('  ')).toBeNull();
    expect(badgeText(undefined)).toBeNull();
  });
  it('defaults the shape to circle', () => {
    expect(badgeShape('hexagon')).toBe('hexagon');
    expect(badgeShape('blob')).toBe('circle');
    expect(badgeShape(null)).toBe('circle');
  });
  it('describes every outline', () => {
    expect(shapeGeometry('circle').tag).toBe('circle');
    expect(shapeGeometry('square').tag).toBe('rect');
    expect(shapeGeometry('diamond').tag).toBe('polygon');
    expect(shapeGeometry('hexagon').attrs['points']).toContain('21.5');
  });
  it('shrinks the font as text grows', () => {
    expect(badgeFontSize(1, 'circle')).toBeGreaterThan(badgeFontSize(3, 'circle'));
    expect(badgeFontSize(2, 'diamond')).toBeLessThan(badgeFontSize(2, 'square'));
    expect(badgeFontSize(0, 'circle')).toBe(badgeFontSize(1, 'circle'));
    expect(badgeFontSize(9, 'circle')).toBe(badgeFontSize(3, 'circle'));
  });
});

describe('resolveGlyph', () => {
  const svg = '<svg viewBox="0 0 10 10"><path d="M0 0h10"/></svg>';
  it('prefers a badge over an icon', () => {
    const g = resolveGlyph({ kind: 'k', badge: '7', icon: 'bug', shape: 'square', color: '#fff' });
    expect(g).toEqual({ type: 'badge', text: '7', shape: 'square', fill: '#ffffff', ink: '#000000' });
    expect(resolveGlyph({ badge: 'x' })).toMatchObject({ fill: '#8a91a3' });
  });
  it('uses built-in icons, custom svg and png', () => {
    expect(resolveGlyph({ icon: 'bug', color: '#ff0000' })).toMatchObject({ type: 'icon', name: 'bug', color: '#ff0000' });
    expect(resolveGlyph({ icon: 'bug' })).toMatchObject({ color: 'currentColor' });
    expect(resolveGlyph({ icon: 'mine' }, [{ name: 'mine', svg, pngDataUrl: null }])).toMatchObject({ type: 'svg' });
    expect(resolveGlyph({ icon: 'mine' }, [{ name: 'mine', svg: null, pngDataUrl: 'data:image/png;base64,AAAA' }])).toMatchObject({ type: 'png' });
    expect(resolveGlyph({ svg })).toMatchObject({ type: 'svg' });
    expect(resolveGlyph({ pngDataUrl: 'data:image/png;base64,AAAA' })).toMatchObject({ type: 'png' });
  });
  it('rejects non-png data urls and unusable svg, falling back to the kind shape', () => {
    expect(resolveGlyph({ kind: 'error', color: '#f00', pngDataUrl: 'data:text/html;base64,AAAA' })).toEqual({ type: 'shape', kind: 'error', color: '#f00' });
    expect(resolveGlyph({ kind: 'x', svg: 'nope' })).toMatchObject({ type: 'shape' });
    expect(resolveGlyph({ kind: 'x', icon: 'missing' }, [{ name: 'other', svg, pngDataUrl: null }])).toMatchObject({ type: 'shape' });
    expect(resolveGlyph({ icon: 'bad' }, [{ name: 'bad', svg: null, pngDataUrl: 'javascript:alert(1)' }])).toMatchObject({ type: 'shape', kind: '' });
  });
});

describe('sanitizeSvg', () => {
  const xml = (el: SVGSVGElement | null) => new XMLSerializer().serializeToString(el!);

  it('keeps allowlisted drawing elements and sizes the root to its box', () => {
    const el = sanitizeSvg('<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><title>t</title><g><path d="M0 0L5 5" stroke="red"/><circle cx="1" cy="1" r="1"/></g></svg>')!;
    const out = xml(el);
    expect(out).toContain('<path d="M0 0L5 5" stroke="red"');
    expect(out).toContain('viewBox="0 0 24 24"');
    expect(el.getAttribute('width')).toBe('100%');
    expect(el.getAttribute('aria-hidden')).toBe('true');
    expect(out).toContain('<title>t</title>');
  });
  it('drops scripts, handlers, links, styles and url() values', () => {
    const el = sanitizeSvg(
      '<svg viewBox="0 0 8 8" onload="x()"><script>alert(1)</script><foreignObject><div/></foreignObject><style>*{}</style><a href="javascript:x"><path d="M0 0"/></a><path d="M1 1" fill="url(http://evil/x)" onclick="y()" stroke="#000"/><image href="http://e/x.png"/></svg>',
    )!;
    const out = xml(el);
    expect(out).not.toMatch(/script|foreignObject|style|onload|onclick|href|url\(|image|<a/);
    expect(out).toContain('stroke="#000"');
  });
  it('rejects markup that is not an svg or is malformed', () => {
    expect(sanitizeSvg('<p>hi</p>')).toBeNull();
    expect(sanitizeSvg('<svg><path></svg>')).toBeNull();
    expect(sanitizeSvg('')).toBeNull();
  });
  it('bounds depth and node count', () => {
    expect(sanitizeSvg('<svg>' + '<g>'.repeat(12) + '</g>'.repeat(12) + '</svg>')).not.toBeNull();
    expect(xml(sanitizeSvg('<svg>' + '<g>'.repeat(12) + '<path d="M0 0"/>' + '</g>'.repeat(12) + '</svg>'))).not.toContain('path');
    expect(xml(sanitizeSvg('<svg>' + '<path d="M0 0"/>'.repeat(500) + '</svg>')).match(/<path/g)).toHaveLength(399);
  });
  it('returns null when the parser itself throws', () => {
    const spy = vi.spyOn(DOMParser.prototype, 'parseFromString').mockImplementation(() => {
      throw new Error('boom');
    });
    expect(sanitizeSvg('<svg/>')).toBeNull();
    spy.mockRestore();
  });
});

describe('built-in icons', () => {
  it('ships the 36 names the engine accepts, each a non-empty path', () => {
    expect(BUILTIN_ICONS.map((i) => i.name)).toEqual([
      'alert', 'bug', 'flag', 'star', 'pin', 'bookmark', 'info', 'check', 'x-circle', 'user', 'users', 'lock', 'key', 'shield', 'database', 'network', 'cloud', 'server',
      'credit-card', 'cart', 'mail', 'clock', 'timer', 'rocket', 'power', 'restart', 'cpu', 'memory', 'disk', 'file', 'download', 'upload', 'link', 'zap', 'fire', 'heart',
    ]);
    for (const i of BUILTIN_ICONS) expect(i.d).toMatch(/^M[\d\s.,MmLlHhVvAaCcQqSsZz-]+$/);
  });
});
