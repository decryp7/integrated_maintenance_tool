import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { RovingTabindex, rovingTarget } from './roving-tabindex';

describe('rovingTarget', () => {
  it('steps with the arrows of the axis and stops at the ends', () => {
    expect(rovingTarget(3, 0, 'ArrowDown', 'vertical')).toBe(1);
    expect(rovingTarget(3, 2, 'ArrowDown', 'vertical')).toBeNull();
    expect(rovingTarget(3, 1, 'ArrowUp', 'vertical')).toBe(0);
    expect(rovingTarget(3, 0, 'ArrowUp', 'vertical')).toBeNull();
    expect(rovingTarget(3, 0, 'ArrowRight', 'horizontal')).toBe(1);
    expect(rovingTarget(3, 1, 'ArrowLeft', 'horizontal')).toBe(0);
  });

  it('ignores the other axis, other keys and empty groups', () => {
    expect(rovingTarget(3, 0, 'ArrowRight', 'vertical')).toBeNull();
    expect(rovingTarget(3, 0, 'ArrowDown', 'horizontal')).toBeNull();
    expect(rovingTarget(3, 0, 'a', 'both')).toBeNull();
    expect(rovingTarget(0, 0, 'ArrowDown', 'both')).toBeNull();
    expect(rovingTarget(3, 0, 'ArrowRight', 'both')).toBe(1);
    expect(rovingTarget(3, 2, 'ArrowLeft', 'both')).toBe(1);
  });

  it('jumps with Home and End', () => {
    expect(rovingTarget(4, 2, 'Home', 'vertical')).toBe(0);
    expect(rovingTarget(4, 1, 'End', 'vertical')).toBe(3);
    expect(rovingTarget(4, 3, 'End', 'vertical')).toBeNull();
  });
});

@Component({
  imports: [RovingTabindex],
  template: `<div debriefRoving="button" rovingAxis="vertical"><button id="a">A</button><button id="b">B</button><button id="c" disabled>C</button><button id="d">D</button></div>`,
})
class Host {}

describe('RovingTabindex', () => {
  const setup = async () => {
    const fixture = TestBed.createComponent(Host);
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  };
  const tabs = (root: HTMLElement) => Array.from(root.querySelectorAll('button')).map((b) => b.getAttribute('tabindex'));

  it('makes the first item the only Tab stop (disabled items are left alone)', async () => {
    const root = await setup();
    expect(tabs(root)).toEqual(['0', '-1', null, '-1']);
  });

  it('moves the stop with the arrows, skipping disabled items, and keeps it on the last focused item', async () => {
    const root = await setup();
    const key = (id: string, k: string) => root.querySelector<HTMLElement>('#' + id)!.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
    root.querySelector<HTMLElement>('#a')!.focus();
    key('a', 'ArrowDown');
    expect(document.activeElement?.id).toBe('b');
    expect(tabs(root)).toEqual(['-1', '0', null, '-1']);
    key('b', 'ArrowDown');
    expect(document.activeElement?.id).toBe('d');
    key('d', 'Home');
    expect(document.activeElement?.id).toBe('a');
    expect(tabs(root)).toEqual(['0', '-1', null, '-1']);
  });

  it('leaves keys with a modifier to the browser', async () => {
    const root = await setup();
    root.querySelector<HTMLElement>('#a')!.focus();
    const e = new KeyboardEvent('keydown', { key: 'ArrowDown', ctrlKey: true, bubbles: true, cancelable: true });
    root.querySelector('#a')!.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
    expect(document.activeElement?.id).toBe('a');
  });
});
