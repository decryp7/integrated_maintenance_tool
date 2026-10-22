import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, el, settle } from '../../testing/helpers';
import { VirtualList } from './virtual-list';
import { at } from '../../testing/must';

@Component({
  imports: [VirtualList],
  template: `
    <debrief-virtual-list [items]="items()" [itemHeight]="24" [maxHeight]="240" label="Things">
      <ng-template let-item let-i="index"><span class="cell">{{ i }}:{{ item }}</span></ng-template>
    </debrief-virtual-list>
  `,
})
class Host {
  readonly items = signal<string[]>(Array.from({ length: 10_000 }, (_, i) => `v${i}`));
}

describe('VirtualList', () => {
  async function render() {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    return fixture;
  }

  it('renders only the rows in view for 10,000 items', async () => {
    const fixture = await render();
    const cells = all(fixture, '.cell');
    expect(cells.length).toBeLessThan(25);
    expect(cells[0]?.textContent).toBe('0:v0');
    expect(el(fixture, '.sizer').style.height).toBe('240000px');
    expect(el(fixture, '.vl').style.height).toBe('240px');
  });

  it('exposes list semantics with position and size', async () => {
    const fixture = await render();
    expect(el(fixture, '[role=list]').getAttribute('aria-label')).toBe('Things');
    const first = at(all(fixture, '[role=listitem]'), 0);
    expect(first.getAttribute('aria-posinset')).toBe('1');
    expect(first.getAttribute('aria-setsize')).toBe('10000');
  });

  it('renders the rows that scroll into view, and the last one at the end', async () => {
    const fixture = await render();
    const vl = el(fixture, '.vl');
    vl.scrollTop = 24 * 5000;
    vl.dispatchEvent(new Event('scroll'));
    await settle(fixture);
    const texts = all(fixture, '.cell').map((c) => c.textContent);
    expect(texts).toContain('5000:v5000');
    expect(texts).not.toContain('0:v0');
    vl.scrollTop = 24 * 10_000;
    vl.dispatchEvent(new Event('scroll'));
    await settle(fixture);
    expect(all(fixture, '.cell').at(-1)?.textContent).toBe('9999:v9999');
  });

  it('shrinks to its content when there are few items, and follows changes to the items', async () => {
    const fixture = await render();
    fixture.componentInstance.items.set(['a', 'b']);
    await settle(fixture);
    expect(el(fixture, '.vl').style.height).toBe('48px');
    expect(all(fixture, '.cell')).toHaveLength(2);
    fixture.componentInstance.items.set([]);
    await settle(fixture);
    expect(all(fixture, '.cell')).toHaveLength(0);
  });
});
