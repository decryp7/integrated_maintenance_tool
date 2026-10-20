import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { HEAP_TABS, HeapSelection, sparkline } from './heap-selection';

describe('HeapSelection', () => {
  const make = () => {
    TestBed.configureTestingModule({ providers: [HeapSelection] });
    return TestBed.inject(HeapSelection);
  };

  it('starts on the findings tab with nothing selected', () => {
    const s = make();
    expect([s.tab(), s.typeName(), s.object()]).toEqual(['findings', null, null]);
    expect(HEAP_TABS.map((t) => t.id)).toEqual(['findings', 'types', 'diff', 'growth', 'dominators', 'instances']);
  });

  it('showInstances selects the type, clears the object and switches tab', () => {
    const s = make();
    s.showPaths(5, 'Old');
    s.showInstances('Foo');
    expect([s.tab(), s.typeName(), s.object()]).toEqual(['instances', 'Foo', null]);
  });

  it('showPaths selects the object and optionally the type', () => {
    const s = make();
    s.showInstances('Foo');
    s.showPaths(7);
    expect([s.tab(), s.typeName(), s.object()]).toEqual(['instances', 'Foo', 7]);
    s.showPaths(8, 'Bar');
    expect([s.typeName(), s.object()]).toEqual(['Bar', 8]);
  });
});

describe('sparkline', () => {
  it('is empty without values', () => {
    expect(sparkline([])).toBe('');
  });
  it('puts a single value in the middle at x = 0', () => {
    expect(sparkline([5])).toBe('0.0,10.0');
  });
  it('puts a flat series in the middle', () => {
    expect(sparkline([3, 3, 3], 80, 20)).toBe('0.0,10.0 40.0,10.0 80.0,10.0');
  });
  it('maps min to the bottom and max to the top, inside a 1px margin', () => {
    expect(sparkline([0, 10], 80, 20)).toBe('0.0,19.0 80.0,1.0');
    expect(sparkline([0, 5, 10], 10, 10)).toBe('0.0,9.0 5.0,5.0 10.0,1.0');
  });
  it('uses the default 80 x 20 box', () => {
    expect(sparkline([1, 2])).toBe('0.0,19.0 80.0,1.0');
  });
});
