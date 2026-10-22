import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { MatIconRegistry } from '@angular/material/icon';
import { firstValueFrom } from 'rxjs';
import { el, settle } from '../../testing/helpers';
import { Icon } from './icon';
import { BLANK_ICON, ICON_PATHS, DEBRIEF_ICON_NAMESPACE, DebriefIconSet, iconSvg, isDebriefIcon, provideDebriefIcons } from './material/debrief-icons';

@Component({
  imports: [Icon],
  template: `<debrief-icon id="i" [name]="name()" [size]="size()" [stroke]="stroke()" [label]="label()" />`,
})
class Host {
  readonly name = signal('search');
  readonly size = signal(16);
  readonly stroke = signal(1.7);
  readonly label = signal('');
}

const NAMES = Object.keys(ICON_PATHS);

describe('debrief icon set', () => {
  it('builds a stroke-only, currentColor SVG with no stroke width baked in', () => {
    const svg = iconSvg('M1 1h2');
    expect(svg).toContain('viewBox="0 0 24 24"');
    expect(svg).toContain('fill="none"');
    expect(svg).toContain('stroke="currentColor"');
    expect(svg).toContain('<path d="M1 1h2"/>');
    expect(svg).not.toContain('stroke-width');
    expect(iconSvg('')).not.toContain('<path');
  });

  it('knows its own names only', () => {
    expect(isDebriefIcon('timeline')).toBe(true);
    expect(isDebriefIcon('nope')).toBe(false);
    expect(isDebriefIcon('toString')).toBe(false);
  });

  it('registers every icon (and the blank fallback) in the debrief namespace', async () => {
    const set = TestBed.inject(DebriefIconSet);
    const registry = TestBed.inject(MatIconRegistry);
    for (const name of [...NAMES, BLANK_ICON]) {
      const svg = await firstValueFrom(registry.getNamedSvgIcon(name, DEBRIEF_ICON_NAMESPACE));
      expect(svg.tagName.toLowerCase()).toBe('svg');
      expect(svg.getAttribute('stroke')).toBe('currentColor');
      expect(svg.querySelector('path')?.getAttribute('d') ?? '').toBe(ICON_PATHS[name] ?? '');
    }
    expect(set.svgIcon('files')).toBe('debrief:files');
    expect(set.svgIcon('nope')).toBe(`debrief:${BLANK_ICON}`);
  });

  it('registers eagerly through provideDebriefIcons', () => {
    const spy = vi.spyOn(MatIconRegistry.prototype, 'addSvgIconLiteralInNamespace');
    TestBed.configureTestingModule({ providers: [provideDebriefIcons()] });
    TestBed.inject(MatIconRegistry);
    expect(spy).toHaveBeenCalledTimes(NAMES.length + 1);
    expect(spy).toHaveBeenCalledWith(DEBRIEF_ICON_NAMESPACE, 'timeline', expect.anything());
    spy.mockRestore();
  });
});

describe('<debrief-icon>', () => {
  it.each(NAMES)('renders %s as an inline svg', async (name) => {
    const f = TestBed.createComponent(Host);
    f.componentInstance.name.set(name);
    await settle(f);
    const path = el(f, '#i mat-icon svg path');
    expect(path.getAttribute('d')).toBe(ICON_PATHS[name]);
  });

  it('renders an empty icon of the same size for unknown names, without errors', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const f = TestBed.createComponent(Host);
    f.componentInstance.name.set('nope');
    f.componentInstance.size.set(20);
    await settle(f);
    expect(el(f, '#i mat-icon svg')).toBeTruthy();
    expect(f.nativeElement.querySelector('#i path')).toBeNull();
    expect(el(f, '#i mat-icon').style.width).toBe('20px');
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  it('sizes the icon from the size input and strokes it from the stroke input', async () => {
    const f = TestBed.createComponent(Host);
    await settle(f);
    const icon = el(f, '#i mat-icon');
    expect(icon.style.width).toBe('16px');
    expect(icon.style.height).toBe('16px');
    expect(icon.style.strokeWidth).toBe('1.7');
    f.componentInstance.size.set(24);
    f.componentInstance.stroke.set(3);
    await settle(f);
    expect(icon.style.width).toBe('24px');
    expect(icon.style.height).toBe('24px');
    expect(icon.style.strokeWidth).toBe('3');
    const svg = el(f, '#i svg');
    expect(svg.getAttribute('width')).toBe('100%');
    expect(svg.getAttribute('height')).toBe('100%');
  });

  it('is decorative unless labelled', async () => {
    const f = TestBed.createComponent(Host);
    await settle(f);
    const icon = el(f, '#i mat-icon');
    expect(icon.getAttribute('aria-hidden')).toBe('true');
    expect(icon.hasAttribute('aria-label')).toBe(false);
    expect(icon.hasAttribute('role')).toBe(false);
    f.componentInstance.label.set('Search');
    await settle(f);
    expect(icon.hasAttribute('aria-hidden')).toBe(false);
    expect(icon.getAttribute('aria-label')).toBe('Search');
    expect(icon.getAttribute('role')).toBe('img');
  });

  it('swaps the svg when the name changes', async () => {
    const f = TestBed.createComponent(Host);
    await settle(f);
    expect(el(f, '#i path').getAttribute('d')).toContain('M10.5');
    f.componentInstance.name.set('plus');
    await settle(f);
    expect(el(f, '#i path').getAttribute('d')).toBe(ICON_PATHS['plus']);
  });
});
