import { cloneForSnapshot, documentCss, inheritedStyle, Rasterize, snapshotDocument, snapshotPanel, snapshotScale, snapshotSvg, SNAPSHOT_MAX_BYTES, svgUrl } from './panel-snapshot';

const PNG = 'data:image/png;base64,iVBORw0KGgo=';

function panel(html: string, width = 800, height = 600): HTMLElement {
  const el = document.createElement('div');
  el.innerHTML = html;
  el.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, width, height, right: width, bottom: height, toJSON: () => ({}) }) as DOMRect;
  document.body.append(el);
  return el;
}

describe('panel snapshots', () => {
  afterEach(() => (document.body.innerHTML = ''));

  it('fits the panel into the ghost without enlarging it', () => {
    expect(snapshotScale({ width: 1120, height: 420 }, { width: 560, height: 420 })).toBe(0.5);
    expect(snapshotScale({ width: 100, height: 100 }, { width: 560, height: 420 })).toBe(1);
    expect(snapshotScale({ width: 0, height: 100 }, { width: 560, height: 420 })).toBe(0);
  });

  it('wraps XHTML in an SVG foreignObject and makes it an image URL', () => {
    const svg = snapshotSvg('<div xmlns="http://www.w3.org/1999/xhtml">a&amp;b</div>', 200.4, 99.6);
    expect(svg).toContain('width="200" height="100"');
    expect(svg).toContain('<foreignObject x="0" y="0" width="200" height="100"><div');
    expect(svgUrl(svg)).toMatch(/^data:image\/svg\+xml;charset=utf-8,%3Csvg/);
  });

  it('copies the panel without scripts, frames, dock overlays or event handlers', () => {
    const el = panel('<p onclick="x()">text</p><script>bad()</script><iframe></iframe><div class="dv-drop-target-dropzone"></div><canvas width="20" height="10"></canvas>');
    const clone = cloneForSnapshot(el)!;
    expect(clone.querySelector('script, iframe, .dv-drop-target-dropzone, canvas')).toBeNull();
    expect(clone.querySelector('p')!.hasAttribute('onclick')).toBe(false);
    // The canvas became a picture of itself (empty where the canvas cannot be read).
    expect(clone.querySelector('img')).not.toBeNull();
    expect(el.querySelector('script')).not.toBeNull();
    expect(cloneForSnapshot(panel('<b></b><b></b><b></b>'), 2)).toBeNull();
  });

  it('carries the theme and the page styles into the snapshot document', () => {
    document.documentElement.setAttribute('data-scheme', 'dark');
    const style = document.createElement('style');
    style.textContent = '.x { color: red; }';
    document.head.append(style);
    const el = panel('<span class="x">hi</span>');
    el.style.setProperty('--panel-bg', '#123456');
    expect(documentCss(document)).toContain('.x');
    expect(inheritedStyle(el)).toContain('color');
    const xhtml = snapshotDocument(el, cloneForSnapshot(el)!, 300, 200);
    expect(xhtml).toContain('xmlns="http://www.w3.org/1999/xhtml"');
    expect(xhtml).toContain('data-scheme="dark"');
    expect(xhtml).toContain('width:300px;height:200px');
    expect(xhtml).toContain('.x');
    style.remove();
    document.documentElement.removeAttribute('data-scheme');
  });

  it('renders through the rasterizer at the fitting scale, halves it once when too large, and never throws', async () => {
    const el = panel('<p>hello</p>', 1120, 840);
    const scales: number[] = [];
    const ok: Rasterize = async (url, w, h, s) => {
      expect(url).toMatch(/^data:image\/svg\+xml/);
      expect([w, h]).toEqual([1120, 840]);
      scales.push(s);
      return PNG;
    };
    expect(await snapshotPanel(el, { width: 560, height: 420 }, ok)).toBe(PNG);
    expect(scales).toEqual([0.5]);
    const big = `data:image/png;base64,${'A'.repeat(SNAPSHOT_MAX_BYTES)}`;
    const sizes: number[] = [];
    const tooBig: Rasterize = async (_u, _w, _h, s) => (sizes.push(s), s < 0.5 ? PNG : big);
    expect(await snapshotPanel(el, { width: 560, height: 420 }, tooBig)).toBe(PNG);
    expect(sizes).toEqual([0.5, 0.25]);
    expect(await snapshotPanel(el, { width: 560, height: 420 }, async () => big)).toBeNull();
    expect(await snapshotPanel(el, { width: 560, height: 420 }, async () => null)).toBeNull();
    expect(await snapshotPanel(el, { width: 560, height: 420 }, async () => 'data:image/svg+xml,x')).toBeNull();
    expect(
      await snapshotPanel(el, { width: 560, height: 420 }, async () => {
        throw new DOMException('tainted', 'SecurityError');
      }),
    ).toBeNull();
    // Too slow: given up.
    expect(await snapshotPanel(el, { width: 560, height: 420 }, () => new Promise(() => undefined), 5)).toBeNull();
    expect(await snapshotPanel(null, { width: 560, height: 420 }, ok)).toBeNull();
    expect(await snapshotPanel(panel('', 0, 0), { width: 560, height: 420 }, ok)).toBeNull();
  });

  it('the default rasterizer gives up quietly where the webview cannot draw', async () => {
    // jsdom has no image decoding or canvas: no snapshot, no error.
    expect(await snapshotPanel(panel('<p>x</p>'), { width: 560, height: 420 }, undefined, 50)).toBeNull();
  });
});
