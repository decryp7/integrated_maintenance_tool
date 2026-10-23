import { describe, expect, it, vi } from 'vitest';
import { MockServiceHealth } from './mock-service-health.backend';
import { TauriServiceHealthBackend } from './tauri-service-health.backend';
import { SERVICE_HEALTH_EVENT } from '../../health/service-health.types';
import { NoServices } from '../../health/service-health.backend';

describe('TauriServiceHealthBackend', () => {
  it('sends the commands of the contract', async () => {
    const call = vi.fn(async () => ({ revision: 1, services: [] }) as never);
    const b = new TauriServiceHealthBackend(call, async () => () => undefined);
    await b.snapshot();
    await b.repair('store');
    expect(call.mock.calls).toEqual([['health_snapshot'], ['health_repair', { service: 'store' }]]);
  });

  it('listens to the service-health event and ignores payloads of another shape', async () => {
    let deliver: (p: unknown) => void = () => undefined;
    const listen = vi.fn(async (event: string, h: (p: unknown) => void) => {
      deliver = h;
      expect(event).toBe(SERVICE_HEALTH_EVENT);
      return () => undefined;
    });
    const b = new TauriServiceHealthBackend(vi.fn(), listen);
    const seen = vi.fn();
    await b.onChange(seen);
    deliver({ nope: true });
    deliver(null);
    deliver({ revision: 4, services: [] });
    expect(seen).toHaveBeenCalledTimes(1);
  });
});

describe('MockServiceHealth', () => {
  it('knows the five services of the engine, all working', async () => {
    const s = await new MockServiceHealth().snapshot();
    expect(s.services.map((x) => x.service)).toEqual(['store', 'ai', 'plugins', 'browser', 'downloads']);
    expect(s.services.every((x) => x.state === 'ok')).toBe(true);
  });

  it('announces a change once, like the engine, and a failed repair leaves the service down', async () => {
    const m = new MockServiceHealth();
    const seen = vi.fn();
    const off = await m.onChange(seen);
    const cause = { message: 'x', hint: null, detail: null };
    m.fail('store', cause);
    m.fail('store', cause);
    m.fail('nope', cause);
    expect(seen).toHaveBeenCalledTimes(1);
    m.repairSucceeds = false;
    expect((await m.repair('store')).services[0]?.state).toBe('down');
    m.repairSucceeds = true;
    expect((await m.repair('store')).services[0]?.state).toBe('ok');
    off();
    m.fail('store', cause);
    expect(seen).toHaveBeenCalledTimes(2);
  });
});

describe('NoServices', () => {
  it('is an empty, quiet host', async () => {
    const b = new NoServices();
    expect((await b.snapshot()).services).toEqual([]);
    expect((await b.repair()).revision).toBe(1);
    (await b.onChange())();
  });
});
