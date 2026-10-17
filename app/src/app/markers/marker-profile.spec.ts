import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { MarkerPreviewService } from './marker-preview';
import { MarkerProfile } from './marker-profile';
import { writeRows } from './marker-rules';

describe('MarkerProfile', () => {
  const setup = () => {
    const backend = tinyBackend();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    return { backend, profile: TestBed.inject(MarkerProfile) };
  };

  it('loads rows and commits a change through validate and save', async () => {
    const { backend, profile } = setup();
    const { rows } = await profile.load();
    expect(rows.map((r) => r.rule.kind)).toContain('fatal');
    const res = await profile.commit((model, r) => {
      r.push({ rule: { kind: 'mine', badge: '1', when: [{ field: '@message', op: 'matches', value: 'Slow' }] }, enabled: true });
      writeRows(model, r);
    });
    expect(res.ok).toBe(true);
    await settle();
    expect((await backend.getProfile()).toml).toContain('badge = "1"');
    expect(TestBed.inject(ImtBackend)).toBe(backend);
  });

  it('reports engine validation errors with the offending line', async () => {
    const { profile } = setup();
    const res = await profile.commit((model, r) => {
      r.push({ rule: { kind: 'bad', badge: 'TOOLONG' }, enabled: true });
      writeRows(model, r);
    });
    expect(res).toMatchObject({ ok: false, failure: { message: expect.stringContaining('badge'), line: expect.any(Number), excerpt: expect.stringContaining('TOOLONG') } });
  });

  it('keeps a message without a line when validation gives none, and turns exceptions into failures', async () => {
    const { backend, profile } = setup();
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: null, line: null });
    expect(await profile.commit(() => undefined)).toEqual({ ok: false, failure: { message: 'The profile is not valid.', line: null, excerpt: null } });
    vi.spyOn(backend, 'saveProfile').mockRejectedValueOnce('disk full');
    expect(await profile.commit(() => undefined)).toMatchObject({ ok: false, failure: { message: 'disk full' } });
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce(new Error('gone'));
    expect(await profile.commit(() => undefined)).toMatchObject({ ok: false, failure: { message: 'gone' } });
  });
});

describe('MarkerPreviewService', () => {
  it('counts, samples and buckets the matches of conditions or a query', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const svc = TestBed.inject(MarkerPreviewService);
    const r = await svc.preview({ conditions: [{ field: '@level', op: 'eq', value: 'Error' }, { field: 'x', op: 'in', value: 'a,b' }] });
    expect(r.count).toBe(1);
    expect(r.samples).toHaveLength(1);
    expect(r.strip.reduce((a, b) => a + b, 0)).toBe(1);
    expect(r.ignored).toBe(1);
    expect(svc.queryFor({ conditions: [], ql: ' level = error ' }).ql).toBe('level = error');
    expect((await svc.preview({ conditions: [], ql: 'x' })).ignored).toBe(0);
  });
});
