/** jsdom gaps the app relies on. Loaded before every spec (angular.json test.setupFiles). */
import '@angular/localize/init';
import * as mocks from './app/mock-backends';
import { installBrowserMocks } from './app/core/mock-registry';

// The app loads the mock backends before bootstrapping in a browser; specs get them at once.
installBrowserMocks(mocks);

// Specs that stand in for Tauri set its window internals; spec files can share a window, so a leftover stub would make
// the next spec believe it runs inside Tauri. Every spec starts and ends in a plain browser.
const dropTauriStub = () => delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
beforeEach(dropTauriStub);
afterEach(dropTauriStub);

class ResizeObserverStub {
  constructor(private readonly cb: ResizeObserverCallback) {}
  observe(target: Element): void {
    // Report a (zero-sized) entry so libraries that read `entry.target` (dockview) keep working.
    this.cb([{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry], this as unknown as ResizeObserver);
  }
  unobserve(): void {}
  disconnect(): void {}
}

if (!('ResizeObserver' in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}

if (!window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => undefined;
  Element.prototype.releasePointerCapture = () => undefined;
}

if (!('PointerEvent' in globalThis)) {
  class PointerEventPolyfill extends MouseEvent {
    readonly pointerId: number;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
    }
  }
  (globalThis as unknown as { PointerEvent: unknown }).PointerEvent = PointerEventPolyfill;
}
