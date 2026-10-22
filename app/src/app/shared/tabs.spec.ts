import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, el, settle } from '../../testing/helpers';
import { TabItem, Tabs } from './tabs';

@Component({
  imports: [Tabs],
  template: `<imt-tabs label="Demo" idPrefix="demo-tab-" [kind]="kind()" [items]="items" [selected]="sel()" [panelId]="panel" (selectedChange)="sel.set($event); picked.push($event)" />`,
})
class Host {
  readonly kind = signal<'tabs' | 'filter'>('tabs');
  readonly sel = signal('a');
  readonly picked: string[] = [];
  readonly items: TabItem[] = [
    { id: 'a', label: 'Alpha' },
    { id: 'b', label: 'Beta', count: 3 },
    { id: 'c', label: 'Gamma', count: 0 },
  ];
  readonly panel = (id: string) => 'panel-' + id;
}

const key = (k: string) => new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });

describe('Tabs', () => {
  async function render() {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    return { fixture, host: fixture.componentInstance, tabs: () => all<HTMLButtonElement>(fixture, 'button') };
  }

  it('is an ARIA tab list: one tab stop, ids and the panel they control, counts only when above zero', async () => {
    const { fixture, tabs } = await render();
    expect(el(fixture, 'imt-tabs').getAttribute('role')).toBe('tablist');
    expect(tabs().map((t) => t.getAttribute('role'))).toEqual(['tab', 'tab', 'tab']);
    expect(tabs().map((t) => t.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false']);
    expect(tabs().map((t) => t.getAttribute('tabindex'))).toEqual([null, '-1', '-1']);
    expect(tabs()[1]?.id).toBe('demo-tab-b');
    expect(tabs()[1]?.getAttribute('aria-controls')).toBe('panel-b');
    expect(tabs().map((t) => t.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Alpha', 'Beta3', 'Gamma']);
  });

  it('selects on click (once) and moves with the arrow, Home and End keys, wrapping', async () => {
    const { fixture, host, tabs } = await render();
    tabs()[0]!.click();
    expect(host.picked).toEqual([]);
    tabs()[1]!.click();
    expect(host.picked).toEqual(['b']);
    tabs()[1]!.dispatchEvent(key('ArrowRight'));
    tabs()[2]!.dispatchEvent(key('ArrowRight'));
    tabs()[0]!.dispatchEvent(key('ArrowLeft'));
    tabs()[0]!.dispatchEvent(key('End'));
    tabs()[2]!.dispatchEvent(key('Home'));
    tabs()[1]!.dispatchEvent(key('x'));
    await settle(fixture);
    expect(host.picked).toEqual(['b', 'c', 'a', 'c', 'c', 'a']);
  });

  it('as a filter is a group of toggle buttons: pressed state, no roles, arrows only move focus', async () => {
    const { fixture, host, tabs } = await render();
    host.kind.set('filter');
    await settle(fixture);
    expect(el(fixture, 'imt-tabs').getAttribute('role')).toBe('group');
    expect(tabs().map((t) => t.getAttribute('aria-pressed'))).toEqual(['true', 'false', 'false']);
    expect(tabs()[0]?.getAttribute('role')).toBeNull();
    expect(tabs()[0]?.hasAttribute('id')).toBe(false);
    tabs()[0]!.dispatchEvent(key('ArrowRight'));
    await settle(fixture);
    expect(host.picked).toEqual([]);
  });
});
