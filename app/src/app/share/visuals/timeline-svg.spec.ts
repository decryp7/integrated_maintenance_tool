import { Timeline } from '../../core/api.types';
import { escapeXml, safeColor, SVG_PALETTES, timelineToSvg } from './timeline-svg';

const MIN = 60_000;

function timeline(patch: Partial<Timeline> = {}): Timeline {
  return {
    from: 0,
    to: 60 * MIN,
    bucketMs: MIN,
    histogram: [],
    lanes: [
      {
        key: 'WEB01 · <3312> & "x"',
        first: 0,
        last: 60 * MIN,
        total: 10,
        matched: 6,
        density: [0, 2, 4, 0],
        markers: [
          { kind: 'error', bucket: 1, ts: 10 * MIN, id: 4, count: 1, message: 'Order <b>failed</b>' },
          { kind: 'crash', bucket: 2, ts: 30 * MIN, id: 9, count: 3, message: 'boom' },
        ],
      },
      { key: 'a-very-long-instance-key-that-is-shortened-in-the-label', first: 0, last: 1, total: 1, matched: 0, density: [], markers: [] },
    ],
    truncatedLanes: 0,
    ...patch,
  };
}

/** Parses the SVG with the browser's XML parser (it must be well-formed). */
function parse(svg: string): Document {
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
  expect(doc.getElementsByTagName('parsererror')).toHaveLength(0);
  return doc;
}

describe('timelineToSvg', () => {
  it('draws title, axis, lanes, density bars and markers as well-formed SVG', () => {
    const svg = timelineToSvg({ timeline: timeline(), title: 'Timeline <demo>', markerColors: { error: '#f2545b', crash: 'red;evil' }, theme: 'light', tz: 'utc', width: 800 });
    const doc = parse(svg);
    const root = doc.documentElement;
    expect(root.getAttribute('width')).toBe('800');
    expect(doc.querySelector('title')?.textContent).toBe('Timeline <demo>');
    expect(root.querySelectorAll('g.lane')).toHaveLength(2);
    expect(root.querySelectorAll('g.axis text').length).toBeGreaterThan(1);
    // Two non-empty density buckets in the first lane.
    expect(root.querySelectorAll('g.lane')[0]?.querySelectorAll('rect')).toHaveLength(2);
    const circles = root.querySelectorAll('circle');
    expect(circles).toHaveLength(2);
    expect(circles[0]?.getAttribute('fill')).toBe('#f2545b');
    // An unsafe colour falls back to the theme.
    expect(circles[1]?.getAttribute('fill')).toBe(SVG_PALETTES.light.muted);
    expect(circles[1]?.getAttribute('r')).toBe('5');
    expect(circles[1]?.querySelector('title')?.textContent).toBe('crash ×3: boom');
    expect(svg).not.toContain('<b>');
    expect(svg).toContain('Order &lt;b&gt;failed&lt;/b&gt;');
    const labels = [...root.querySelectorAll('g.lane > text')].map((t) => t.textContent);
    expect(labels[1]?.endsWith('…')).toBe(true);
    expect(labels[0]).toContain('WEB01 · <3312> & "x"');
    expect(svg).not.toMatch(/<(script|foreignObject|style)/);
  });

  it('draws annotations, the truncation note and the dark palette', () => {
    const svg = timelineToSvg({
      timeline: timeline({ truncatedLanes: 2 }),
      title: 'T',
      annotations: [
        { from: 5 * MIN, to: 20 * MIN, label: 'reboot', color: '#3987e5' },
        { from: 40 * MIN, to: 40 * MIN, label: 'point', color: null },
      ],
      theme: 'dark',
      tz: 'utc',
    });
    const doc = parse(svg);
    const ann = doc.querySelectorAll('g.annotations rect');
    expect(ann).toHaveLength(2);
    expect(ann[0]?.getAttribute('fill')).toBe('#3987e5');
    expect(ann[1]?.getAttribute('fill')).toBe(SVG_PALETTES.dark.accent);
    expect(Number(ann[1]?.getAttribute('width'))).toBeGreaterThanOrEqual(2);
    expect(svg).toContain('2 more instances not shown');
    expect(doc.documentElement.querySelector('rect')?.getAttribute('fill')).toBe(SVG_PALETTES.dark.background);
    expect(timelineToSvg({ timeline: timeline({ truncatedLanes: 1 }), title: 'T', theme: 'light', tz: 'local' })).toContain('1 more instance not shown');
  });

  it('handles an empty timeline and narrow widths', () => {
    const svg = timelineToSvg({ timeline: timeline({ lanes: [], from: 5, to: 5 }), title: 'Empty', theme: 'light', tz: 'utc', width: 100 });
    const doc = parse(svg);
    expect(doc.documentElement.getAttribute('width')).toBe('400');
    expect(svg).toContain('No events in this range.');
  });

  it('escapes text and checks colours', () => {
    expect(escapeXml(`<a href="x">'&'</a>`)).toBe('&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;');
    expect(safeColor('#abc', 'x')).toBe('#abc');
    expect(safeColor(' #A1B2C3D4 ', 'x')).toBe('#A1B2C3D4');
    expect(safeColor('url(#x)', 'x')).toBe('x');
    expect(safeColor(null, 'x')).toBe('x');
    expect(safeColor(undefined, 'x')).toBe('x');
  });
});
