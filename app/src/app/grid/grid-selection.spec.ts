import { effect } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { RecordView } from '../core/api.types';
import { GridSelection } from './grid-selection';

const rec = (id: number): RecordView => ({ id, ts: id, tsState: 'parsed', level: 'Information', message: `line ${id}`, template: null, exception: null, fields: {}, source: 0, sourceName: 's.log', line: id, byteOffset: 0, group: 'g', markers: [] });

describe('GridSelection', () => {
  it('starts empty and publishes a copy of the records in order', () => {
    const sel = TestBed.inject(GridSelection);
    expect(sel.records()).toEqual([]);
    expect(sel.count()).toBe(0);
    const records = [rec(3), rec(1)];
    sel.publish(records);
    records.push(rec(9));
    expect(sel.records().map((r) => r.id)).toEqual([3, 1]);
    expect(sel.count()).toBe(2);
    sel.clear();
    expect(sel.records()).toEqual([]);
  });

  it('does not notify readers when the same records are published again', () => {
    const sel = TestBed.inject(GridSelection);
    let runs = 0;
    TestBed.runInInjectionContext(() => effect(() => (sel.records(), runs++)));
    TestBed.tick();
    sel.publish([rec(1), rec(2)]);
    TestBed.tick();
    sel.publish([{ ...rec(1) }, { ...rec(2) }]);
    TestBed.tick();
    expect(runs).toBe(2);
    sel.publish([rec(2), rec(1)]);
    TestBed.tick();
    expect(runs).toBe(3);
  });

  it('answers the records for an action on the selected lines, falling back to one record or none', () => {
    const sel = TestBed.inject(GridSelection);
    expect(sel.recordsOr(null)).toEqual([]);
    expect(sel.recordsOr(rec(5)).map((r) => r.id)).toEqual([5]);
    sel.publish([rec(1), rec(2)]);
    expect(sel.recordsOr(rec(5)).map((r) => r.id)).toEqual([1, 2]);
  });
});
