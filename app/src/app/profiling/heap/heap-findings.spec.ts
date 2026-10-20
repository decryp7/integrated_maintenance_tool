import { Component, inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, settle } from '../../../testing/helpers';
import { MockProfilingBackend } from '../../core/mock/mock-profiling';
import { CapturesStore } from '../captures.store';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { HeapFindings } from './heap-findings';
import { HeapSelection } from './heap-selection';

@Component({
  imports: [HeapFindings],
  providers: [HeapSelection],
  template: `<imt-heap-findings [snapshotId]="id" />`,
})
class Host {
  id = 0;
  readonly sel = inject(HeapSelection);
}

async function mount(which: 'first' | 'last') {
  setupProfiling();
  await openDemoCaptures();
  const heaps = TestBed.inject(CapturesStore).heaps();
  const fixture = TestBed.createComponent(Host);
  fixture.componentInstance.id = (which === 'first' ? heaps[0] : heaps.at(-1))!.id;
  fixture.detectChanges();
  await settle(fixture, 6);
  return fixture;
}

describe('HeapFindings', () => {
  it('lists the biggest retainer and, after a previous snapshot, the growth', async () => {
    const fixture = await mount('last');
    const items = all(fixture, 'li.finding');
    expect(items.map((i) => i.getAttribute('data-kind'))).toEqual(['topRetainers', 'growth']);
    // The retainer opens the dominator tree; growth opens the comparison.
    (items[0]!.querySelector('button') as HTMLButtonElement).click();
    expect(fixture.componentInstance.sel.tab()).toBe('dominators');
    (items[1]!.querySelector('button') as HTMLButtonElement).click();
    expect(fixture.componentInstance.sel.tab()).toBe('diff');
  });

  it('has no growth for the first snapshot', async () => {
    const fixture = await mount('first');
    expect(all(fixture, 'li.finding').map((i) => i.getAttribute('data-kind'))).toEqual(['topRetainers']);
  });

  it('type findings list the instances of their type', async () => {
    class Backend extends MockProfilingBackend {
      override heapFindings() {
        return Promise.resolve([
          { kind: 'largeObjects' as const, severity: 'warning' as const, title: 'Large', detail: 'd', valueBytes: 1, typeName: 'System.Byte[]', view: 'types' as const },
          { kind: 'staticRetention' as const, severity: 'warning' as const, title: 'Statics', detail: 'd', valueBytes: null, typeName: null, view: 'paths' as const },
        ]);
      }
    }
    setupProfiling({ backend: new Backend() });
    await openDemoCaptures();
    const fixture = TestBed.createComponent(Host);
    fixture.componentInstance.id = TestBed.inject(CapturesStore).heaps()[0]!.id;
    fixture.detectChanges();
    await settle(fixture, 6);
    const buttons = all<HTMLButtonElement>(fixture, 'li.finding button');
    expect(buttons[0]!.textContent).toContain('Show the type');
    buttons[0]!.click();
    expect(fixture.componentInstance.sel.tab()).toBe('instances');
    expect(fixture.componentInstance.sel.typeName()).toBe('System.Byte[]');
    buttons[1]!.click();
    expect(fixture.componentInstance.sel.tab()).toBe('instances');
  });
});
