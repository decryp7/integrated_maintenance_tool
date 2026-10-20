import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, settle } from '../../testing/helpers';
import { AppLogo } from './app-logo';

@Component({
  imports: [AppLogo],
  template: `<imt-app-logo id="a" [size]="size()" [label]="label()" /><imt-app-logo id="b" />`,
})
class Host {
  readonly size = signal(48);
  readonly label = signal('');
}

describe('AppLogo', () => {
  async function render() {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    return fixture;
  }

  it('renders at the requested size and is decorative without a label', async () => {
    const fixture = await render();
    const svg = el<SVGSVGElement>(fixture, '#a svg');
    expect(svg.getAttribute('width')).toBe('48');
    expect(svg.getAttribute('height')).toBe('48');
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.getAttribute('role')).toBeNull();
    expect(el<SVGSVGElement>(fixture, '#b svg').getAttribute('width')).toBe('20');
  });

  it('becomes an image with an accessible name when labelled', async () => {
    const fixture = await render();
    fixture.componentInstance.label.set('IMT');
    await settle(fixture);
    const svg = el<SVGSVGElement>(fixture, '#a svg');
    expect(svg.getAttribute('role')).toBe('img');
    expect(svg.getAttribute('aria-label')).toBe('IMT');
    expect(svg.getAttribute('aria-hidden')).toBeNull();
  });

  it('gives every instance its own gradient and fills the tile with it', async () => {
    const fixture = await render();
    const ids = ['#a', '#b'].map((host) => el(fixture, `${host} linearGradient`).id);
    expect(ids[0]).not.toBe(ids[1]);
    expect(el(fixture, '#a rect').getAttribute('fill')).toBe(`url(#${ids[0]})`);
  });
});
