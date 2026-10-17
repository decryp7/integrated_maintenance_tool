import { Component } from '@angular/core';
import { isTauri } from '../platform';
import { ImtView } from '../../extensions/tokens';

/**
 * Test hook for the error-capture coverage tests: `?imtFault=view:<id>` (or `view:*`) makes that view's component
 * throw when it is created, so a test can check every registered view's failure is contained and logged with the
 * operation that opened it. Browser builds only: the desktop app ignores it.
 */
export function faultyViews(search: string = typeof location === 'undefined' ? '' : location.search, desktop: boolean = isTauri()): string | null {
  if (desktop) return null;
  const fault = new URLSearchParams(search).get('imtFault');
  return fault?.startsWith('view:') ? fault.slice('view:'.length) : null;
}

/** A component that throws when it is created (the panel's error boundary names the view). */
@Component({ selector: 'imt-injected-fault', template: '' })
export class InjectedFault {
  constructor() {
    throw new Error('Injected fault (test hook)');
  }
}

/** `views` with the faulty ones (see `faultyViews`) replaced by a throwing component. */
export function withInjectedFaults(views: ImtView[], faulty: string | null = faultyViews()): ImtView[] {
  if (!faulty) return views;
  return views.map((v) => (faulty === '*' || faulty === v.id ? { ...v, component: async () => InjectedFault } : v));
}
