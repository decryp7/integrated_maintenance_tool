import { DEMO_START, generateDemo, generateExtraSource, renderTemplate } from './mock-data';

describe('mock data', () => {
  it('renders message templates like Serilog {Message:lj}', () => {
    expect(renderTemplate('User {Name} took {Ms:0.00} ms {{literal}} {@Obj} {$Str} {Missing}', { Name: 'ann', Ms: 1.5, Obj: { a: 1 }, Str: 's' })).toBe(
      'User ann took 1.5 ms {literal} {"a":1} s {Missing}',
    );
  });

  it('generates a deterministic incident with a crash and restart', () => {
    const a = generateDemo(1);
    const b = generateDemo(1);
    expect(a.records.length).toBe(b.records.length);
    expect(a.records.length).toBeGreaterThan(10_000);
    expect(a.records.filter((r) => r.level === 'Fatal')).toHaveLength(1);
    expect(a.records.some((r) => r.fields['ProcessId'] === 5532)).toBe(true);
    expect(a.records.every((r) => r.ts === null || r.ts >= DEMO_START)).toBe(true);
    expect(a.sources.map((s) => s.importer)).toContain('csv');
    expect(a.unrecognized).toHaveLength(2);
  });

  it('generates an extra source for browser demos', () => {
    const { source, records } = generateExtraSource(7, 'C:\\logs\\notify.log', 3);
    expect(source).toMatchObject({ id: 7, name: 'notify.log', importer: 'text' });
    expect(records.every((r) => r.source === 7)).toBe(true);
    expect(records[0].message).toContain('Application starting');
    expect(generateExtraSource(8, '', 3).source.name).toBe('');
  });
});
