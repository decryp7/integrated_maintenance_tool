import { Provider } from '@angular/core';
import type { Mock } from 'vitest';
import { ECHARTS_INIT, EChartsInit } from '../app/metrics/echarts-init';

/** A recorded chart instance created by `FakeECharts`. */
export interface FakeChart {
  readonly handlers: Record<string, (e: unknown) => void>;
  readonly setOption: Mock;
  readonly dispatchAction: Mock;
  readonly resize: Mock;
  readonly dispose: Mock;
  /** zrender's handlers (`getZr().on`), e.g. a right-click anywhere on the canvas. */
  readonly zrHandlers: Record<string, (e: unknown) => void>;
  /** Whether a pixel is in the plot (defaults to yes). */
  readonly containPixel: Mock;
  /** Pixel to `[x, y]` values (defaults to `[x * 1000, y]`). */
  readonly convertFromPixel: Mock;
  on(name: string, handler: (e: unknown) => void): void;
  getZr(): { on(name: string, handler: (e: unknown) => void): void };
}

/**
 * Stands in for ECharts in component tests: records every instance `EChart` creates. Provided through the
 * `ECHARTS_INIT` token, so it works regardless of which spec files ran before in the same worker.
 */
export class FakeECharts {
  readonly instances: FakeChart[] = [];
  readonly init = vi.fn(() => {
    const c: FakeChart = {
      handlers: {},
      setOption: vi.fn(),
      dispatchAction: vi.fn(),
      resize: vi.fn(),
      dispose: vi.fn(),
      zrHandlers: {},
      containPixel: vi.fn(() => true),
      convertFromPixel: vi.fn((_finder: unknown, [x, y]: number[]) => [(x ?? 0) * 1000, y ?? 0]),
      on(name, handler) {
        c.handlers[name] = handler;
      },
      getZr() {
        return {
          on(name: string, handler: (e: unknown) => void) {
            c.zrHandlers[name] = handler;
          },
        };
      },
    };
    this.instances.push(c);
    return c;
  });

  /** The provider that makes `EChart` use this fake. */
  get provider(): Provider {
    return { provide: ECHARTS_INIT, useValue: this.init as unknown as EChartsInit };
  }

  reset(): void {
    this.instances.length = 0;
    this.init.mockClear();
  }
}
