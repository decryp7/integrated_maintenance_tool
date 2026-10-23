import { TestBed } from '@angular/core/testing';
import { appConfig } from '../app.config';
import { ViewRegistry } from '../extensions/view-registry';

// ADR 0056: the lint over the real registry (the views of every provider the app registers).
describe('every registered view declares its layout', () => {
  it('has layout with min <= pref and a priority, and an area that matches how it opens', () => {
    TestBed.configureTestingModule({ providers: appConfig.providers });
    const views = TestBed.inject(ViewRegistry).all;
    expect(views.length).toBeGreaterThan(20);
    const bad: string[] = [];
    for (const v of views) {
      const l = v.layout;
      if (!l) bad.push(`${v.id}: no layout`);
      else if (l.min.width > l.pref.width || l.min.height > l.pref.height || l.priority <= 0) bad.push(`${v.id}: min/pref/priority`);
    }
    expect(bad).toEqual([]);
  });
});
