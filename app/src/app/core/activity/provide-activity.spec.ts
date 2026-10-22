import { EnvironmentInjector, runInInjectionContext } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../../testing/helpers';
import { ActivityLog } from './activity-log';
import { invoke } from './ipc';
import { ActivityObservers } from './observers';
import { loadDetail, startActivity } from './provide-activity';

describe('provideActivity', () => {
  it('reads the detail setting, and anything but "detailed" is normal', async () => {
    const reply = (v: unknown) => (async () => v) as unknown as typeof invoke;
    expect(await loadDetail(reply({ activity: { detail: 'detailed' } }))).toBe('detailed');
    expect(await loadDetail(reply({ activity: { detail: 'normal' } }))).toBe('normal');
    expect(await loadDetail(reply({ activity: { detail: 'verbose' } }))).toBe('normal');
    expect(await loadDetail(reply({}))).toBe('normal');
    expect(await loadDetail(reply(null))).toBe('normal');
  });

  it('asks the engine for the diagnostics settings', async () => {
    const call = vi.fn(async () => ({ activity: { detail: 'detailed' } }));
    await loadDetail(call as unknown as typeof invoke);
    expect(call).toHaveBeenCalledWith('diagnostics_settings');
  });

  it('never throws when input tracking cannot be installed', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const log = TestBed.inject(ActivityLog);
    vi.spyOn(log.runtime.triggers, 'install').mockImplementation(() => {
      throw new Error('no document');
    });
    const start = vi.spyOn(TestBed.inject(ActivityObservers), 'start');
    expect(() => runInInjectionContext(TestBed.inject(EnvironmentInjector), () => startActivity())).not.toThrow();
    expect(start).not.toHaveBeenCalled();
  });

  it('installs input tracking and starts the observers, and stops tracking when destroyed', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const log = TestBed.inject(ActivityLog);
    const stop = vi.fn();
    vi.spyOn(log.runtime.triggers, 'install').mockReturnValue(stop);
    const start = vi.spyOn(TestBed.inject(ActivityObservers), 'start').mockImplementation(() => undefined);
    runInInjectionContext(TestBed.inject(EnvironmentInjector), () => startActivity());
    expect(start).toHaveBeenCalledTimes(1);
    TestBed.resetTestingModule();
    expect(stop).toHaveBeenCalled();
  });
});
