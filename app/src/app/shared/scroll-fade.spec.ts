import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { hasMoreBelow, ScrollFade } from './scroll-fade';

@Component({ selector: 'debrief-fade-host', changeDetection: ChangeDetectionStrategy.OnPush, imports: [ScrollFade], template: `<div class="list" debriefScrollFade></div>` })
class Host {}

describe('hasMoreBelow', () => {
  it('is true only while content is cut off at the bottom', () => {
    expect(hasMoreBelow(500, 200, 0)).toBe(true);
    expect(hasMoreBelow(500, 200, 298)).toBe(true);
    expect(hasMoreBelow(500, 200, 300)).toBe(false);
    expect(hasMoreBelow(200, 200, 0)).toBe(false);
  });
});

describe('ScrollFade', () => {
  it('flags a list whose rows continue below the edge, and clears the flag when they fit', async () => {
    const fixture = TestBed.createComponent(Host);
    const list: HTMLElement = fixture.nativeElement.querySelector('.list');
    let scrollHeight = 500;
    Object.defineProperty(list, 'scrollHeight', { get: () => scrollHeight });
    Object.defineProperty(list, 'clientHeight', { get: () => 200 });
    await fixture.whenStable();
    list.dispatchEvent(new Event('scroll'));
    expect(list.hasAttribute('data-more-below')).toBe(true);
    scrollHeight = 200;
    list.dispatchEvent(new Event('scroll'));
    expect(list.hasAttribute('data-more-below')).toBe(false);
  });
});
