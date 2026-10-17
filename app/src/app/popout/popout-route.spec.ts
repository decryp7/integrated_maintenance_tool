import { parsePopoutRoute, popoutHash } from './popout-route';

describe('parsePopoutRoute', () => {
  it('parses a valid hash', () => {
    expect(parsePopoutRoute('#/popout?view=log.table&host=main&label=p-main-1')).toEqual({ view: 'log.table', host: 'main', label: 'p-main-1' });
  });

  it('rejects non-popout hashes', () => {
    for (const h of ['', '#/', '#/other?view=a&host=b&label=c', 'popout?view=a&host=b&label=c', '#/popout']) expect(parsePopoutRoute(h)).toBeNull();
    expect(parsePopoutRoute(undefined as unknown as string)).toBeNull();
  });

  it('rejects missing params', () => {
    expect(parsePopoutRoute('#/popout?host=main&label=p-1')).toBeNull();
    expect(parsePopoutRoute('#/popout?view=a&label=p-1')).toBeNull();
    expect(parsePopoutRoute('#/popout?view=a&host=main')).toBeNull();
  });

  it('rejects invalid view ids', () => {
    for (const v of ['../etc/passwd', 'a/b', 'a b', '.hidden', '-x', '', 'a'.repeat(65)]) {
      expect(parsePopoutRoute(`#/popout?view=${encodeURIComponent(v)}&host=main&label=p-1`)).toBeNull();
    }
  });

  it('rejects invalid labels', () => {
    expect(parsePopoutRoute('#/popout?view=a&host=ma%20in&label=p-1')).toBeNull();
    expect(parsePopoutRoute('#/popout?view=a&host=main&label=p_1')).toBeNull();
    expect(parsePopoutRoute('#/popout?view=a&host=main&label=-p')).toBeNull();
    expect(parsePopoutRoute(`#/popout?view=a&host=main&label=${'x'.repeat(65)}`)).toBeNull();
  });

  it('rejects a label equal to the host', () => {
    expect(parsePopoutRoute('#/popout?view=a&host=main&label=main')).toBeNull();
  });
});

describe('popoutHash', () => {
  it('round-trips', () => {
    const route = { view: 'a.b-c_d', host: 'main', label: 'p-main-2' };
    expect(parsePopoutRoute(popoutHash(route))).toEqual(route);
  });

  it('encodes special characters so they cannot inject params', () => {
    const hash = popoutHash({ view: 'a&host=evil', host: 'main', label: 'p-1' });
    expect(hash).toContain('a%26host%3Devil');
    expect(parsePopoutRoute(hash)).toBeNull();
  });
});
