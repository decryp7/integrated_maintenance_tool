import { Injector, runInInjectionContext } from '@angular/core';
import { FailureReporter, reportBackendFailures, reportFailure } from './failure-reporter';

function injectorWith(reporter?: Partial<FailureReporter>): Injector {
  return Injector.create({ providers: reporter ? [{ provide: FailureReporter, useValue: reporter }] : [] });
}

describe('reportFailure', () => {
  it('calls the reporter when one is provided', () => {
    const panelFailed = vi.fn();
    reportFailure(injectorWith({ panelFailed }), (r) => r.panelFailed('grid', 'x'));
    expect(panelFailed).toHaveBeenCalledWith('grid', 'x');
  });

  it('does nothing without a reporter and swallows errors a reporter throws', () => {
    const report = vi.fn();
    expect(() => reportFailure(injectorWith(), report)).not.toThrow();
    expect(report).not.toHaveBeenCalled();
    const broken = injectorWith({
      panelFailed: () => {
        throw new Error('broken');
      },
    });
    expect(() => reportFailure(broken, (r) => r.panelFailed('grid', 'x'))).not.toThrow();
  });
});

describe('reportBackendFailures', () => {
  it('looks the reporter up at call time, from the injection context or an explicit injector', () => {
    const backendFailed = vi.fn();
    const injector = injectorWith({ backendFailed });
    const onFailure = runInInjectionContext(injector, () => reportBackendFailures());
    onFailure('queryPage', 'boom');
    expect(backendFailed).toHaveBeenCalledWith('queryPage', 'boom');
    reportBackendFailures(injectorWith())('queryPage', 'ignored');
    expect(backendFailed).toHaveBeenCalledTimes(1);
  });
});
