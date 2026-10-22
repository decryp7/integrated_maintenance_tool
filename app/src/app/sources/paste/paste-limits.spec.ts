import { byteLength, nameToSend, overCap, PASTE_MAX_BYTES, safeLogName, uniqueName } from './paste-limits';

describe('paste limits', () => {
  it('caps at 8 MiB', () => {
    expect(PASTE_MAX_BYTES).toBe(8 * 1024 * 1024);
    expect(overCap(PASTE_MAX_BYTES)).toBe(false);
    expect(overCap(PASTE_MAX_BYTES + 1)).toBe(true);
    expect(overCap(0)).toBe(false);
  });

  it('counts UTF-8 bytes, not characters', () => {
    expect(byteLength('')).toBe(0);
    expect(byteLength('abc')).toBe(3);
    expect(byteLength('é')).toBe(2);
    expect(byteLength('日本')).toBe(6);
    expect(byteLength('😀')).toBe(4);
  });

  it('sends a trimmed name, or null for an empty one', () => {
    expect(nameToSend('  web01 errors  ')).toBe('web01 errors');
    expect(nameToSend('   ')).toBeNull();
    expect(nameToSend('')).toBeNull();
  });

  it('makes names safe like the engine', () => {
    expect(safeLogName('web01 errors')).toBe('web01 errors.log');
    expect(safeLogName('C:\\temp\\app.txt')).toBe('app.txt');
    expect(safeLogName('../../etc/passwd')).toBe('passwd.log');
    expect(safeLogName('a<b>:c|d?.JSON')).toBe('abcd.JSON');
    expect(safeLogName('events.ndjson')).toBe('events.ndjson');
    expect(safeLogName('x (1)')).toBe('x (1).log');
    expect(safeLogName('...hidden')).toBe('hidden.log');
    expect(safeLogName('???')).toBe('Pasted log.log');
    expect(safeLogName(null)).toBe('Pasted log.log');
    expect(safeLogName('café-é.csv')).toBe('café-é.csv');
  });

  it('suffixes a taken name before the extension', () => {
    expect(uniqueName('a.log', new Set())).toBe('a.log');
    expect(uniqueName('a.log', new Set(['a.log']))).toBe('a (2).log');
    expect(uniqueName('a.log', new Set(['a.log', 'a (2).log']))).toBe('a (3).log');
    expect(uniqueName('noext', new Set(['noext']))).toBe('noext (2)');
    expect(uniqueName('.hidden', new Set(['.hidden']))).toBe('.hidden (2)');
  });
});
