import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, settle } from '../../testing/helpers';
import { Switch } from './switch';

@Component({
  imports: [Switch],
  template: `
    <div>
      <debrief-switch [checked]="on()" [disabled]="off()" describedBy="help" (checkedChange)="changes.push($event); on.set($event)"
        >Load the model in the background when Debrief starts</debrief-switch
      >
    </div>
    <p id="help">Help</p>
  `,
})
class Host {
  readonly on = signal(false);
  readonly off = signal(false);
  readonly changes: boolean[] = [];
}

describe('Switch', () => {
  async function render() {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    return { fixture, host: fixture.componentInstance, input: el<HTMLInputElement>(fixture, 'input') };
  }

  it('is a labelled native switch that reports changes', async () => {
    const { fixture, host, input } = await render();
    expect(input.type).toBe('checkbox');
    expect(input.getAttribute('role')).toBe('switch');
    expect(input.getAttribute('aria-describedby')).toBe('help');
    expect(el(fixture, `label[for="${input.id}"]`).textContent).toContain('Load the model');
    // Clicking the label toggles it, like any labelled checkbox.
    el<HTMLElement>(fixture, '.text').click();
    await settle(fixture);
    expect(host.changes).toEqual([true]);
    expect(input.checked).toBe(true);
    input.click();
    expect(host.changes).toEqual([true, false]);
  });

  it('can be disabled', async () => {
    const { fixture, host, input } = await render();
    host.off.set(true);
    await settle(fixture);
    expect(input.disabled).toBe(true);
  });
});
