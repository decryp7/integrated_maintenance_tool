import { ChangeDetectionStrategy, Component } from '@angular/core';
import { isTauri } from '../platform';
import { DebriefView } from '../../extensions/tokens';

/**
 * Test hook for the error-capture coverage tests: `?debriefFault=view:<id>` (or `view:*`) makes that view's component
 * throw when it is created, so a test can check every registered view's failure is contained and logged with the
 * operation that opened it. Browser builds only: the desktop app ignores it.
 */
export function faultyViews(search: string = typeof location === 'undefined' ? '' : location.search, desktop: boolean = isTauri()): string | null {
  if (desktop) return null;
  const fault = new URLSearchParams(search).get('debriefFault');
  return fault?.startsWith('view:') ? fault.slice('view:'.length) : null;
}

/** A component that throws when it is created (the panel's error boundary names the view). */
@Component({ selector: 'debrief-injected-fault', template: '', changeDetection: ChangeDetectionStrategy.OnPush })
export class InjectedFault {
  constructor() {
    throw new Error('Injected fault (test hook)');
  }
}

/** `views` with the faulty ones (see `faultyViews`) replaced by a throwing component. */
export function withInjectedFaults(views: DebriefView[], faulty: string | null = faultyViews()): DebriefView[] {
  if (!faulty) return views;
  return views.map((v) => (faulty === '*' || faulty === v.id ? { ...v, component: async () => InjectedFault } : v));
}
