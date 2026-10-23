import { Component, inject, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { ConsumerGate, PANEL_VISIBLE, panelVisible } from './panel-visibility';

describe('ConsumerGate', () => {
  it('is active while a registered consumer is visible', () => {
    const gate = new ConsumerGate();
    expect(gate.active()).toBe(false);
    const a = signal(false);
    const b = signal(true);
    const releases: (() => void)[] = [];
    const ref = { onDestroy: (fn: () => void) => (releases.push(fn), () => undefined) };
    gate.add(a, ref);
    expect(gate.active()).toBe(false);
    a.set(true);
    expect(gate.active()).toBe(true);
    gate.add(b, ref);
    a.set(false);
    expect(gate.active()).toBe(true);
    releases[1]!();
    expect(gate.active()).toBe(false);
    // Two consumers in one panel share its visibility; releasing one keeps the other.
    gate.add(a, ref);
    gate.add(a, ref);
    a.set(true);
    releases[2]!();
    expect(gate.active()).toBe(true);
  });

  it('use() registers the calling view for its lifetime, with its panel visibility', () => {
    const gate = new ConsumerGate();
    const visible = signal(false);
    @Component({ selector: 'debrief-consumer', template: '' })
    class Consumer {
      readonly own = panelVisible();
      constructor() {
        gate.use();
      }
    }
    TestBed.configureTestingModule({ providers: [{ provide: PANEL_VISIBLE, useValue: visible }] });
    const fixture = TestBed.createComponent(Consumer);
    expect(fixture.componentInstance.own).toBe(visible);
    expect(gate.active()).toBe(false);
    visible.set(true);
    expect(gate.active()).toBe(true);
    fixture.destroy();
    expect(gate.active()).toBe(false);
  });

  it('outside a panel a view counts as visible', () => {
    const gate = new ConsumerGate();
    @Component({ selector: 'debrief-plain', template: '' })
    class Plain {
      constructor() {
        gate.use();
        void inject(PANEL_VISIBLE, { optional: true });
      }
    }
    const fixture = TestBed.createComponent(Plain);
    expect(gate.active()).toBe(true);
    fixture.destroy();
  });
});
