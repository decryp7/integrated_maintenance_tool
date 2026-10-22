import { breakAfterSeparators } from './inspector-text';

describe('breakAfterSeparators (K1)', () => {
  it('breaks after dots and slashes, keeping the text intact', () => {
    expect(breakAfterSeparators('Microsoft.Hosting.Lifetime')).toEqual(['Microsoft.', 'Hosting.', 'Lifetime']);
    expect(breakAfterSeparators('/var/log/app.log').join('')).toBe('/var/log/app.log');
    expect(breakAfterSeparators('C:\\logs\\a.log')).toEqual(['C:\\', 'logs\\', 'a.', 'log']);
    expect(breakAfterSeparators('plain')).toEqual(['plain']);
    expect(breakAfterSeparators('')).toEqual(['']);
  });
});
