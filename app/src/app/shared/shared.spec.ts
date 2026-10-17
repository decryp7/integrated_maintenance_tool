import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, settle } from '../../testing/helpers';
import { ClickOutside } from './click-outside';
import { Icon } from './icon';
import { MarkerIcon } from './marker-icon';

@Component({
  imports: [MarkerIcon, Icon, ClickOutside],
  template: `
    <imt-marker-icon id="b" [kind]="kind()" [color]="color()" label="Lbl" />
    <imt-icon id="i" [name]="icon()" [size]="20" />
    <div id="pop" (imtClickOutside)="outside = outside + 1"><span id="inner">x</span></div>
  `,
})
class Host {
  readonly kind = signal('fatal');
  readonly color = signal<string | null>(null);
  readonly icon = signal('search');
  outside = 0;
}

describe('shared components', () => {
  it('renders built-in and custom marker shapes', async () => {
    const f = TestBed.createComponent(Host);
    await settle(f);
    const span = el(f, '#b span');
    expect(span.className).toContain('mk-fatal');
    expect(span.getAttribute('aria-label')).toBe('Lbl');
    expect(span.style.background).toBe('');
    f.componentInstance.kind.set('deploy');
    f.componentInstance.color.set('rgb(1, 2, 3)');
    await settle(f);
    expect(span.className).toContain('mk-custom');
    expect(span.style.background).toContain('rgb(1, 2, 3)');
    f.componentInstance.color.set(null);
    await settle(f);
    expect(span.style.background).toContain('var(--muted)');
  });

  it('renders icons and tolerates unknown names', async () => {
    const f = TestBed.createComponent(Host);
    await settle(f);
    expect(el(f, '#i svg').getAttribute('width')).toBe('20');
    expect(el(f, '#i path').getAttribute('d')).toContain('M10.5');
    f.componentInstance.icon.set('nope');
    await settle(f);
    expect(el(f, '#i path').getAttribute('d')).toBe('');
  });

  it('detects clicks outside and Escape', async () => {
    const f = TestBed.createComponent(Host);
    document.body.appendChild(f.nativeElement);
    await settle(f);
    el(f, '#inner').dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(f.componentInstance.outside).toBe(0);
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(f.componentInstance.outside).toBe(1);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(f.componentInstance.outside).toBe(2);
    f.nativeElement.remove();
  });
});
