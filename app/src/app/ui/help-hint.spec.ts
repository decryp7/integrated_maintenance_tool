import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, settle } from '../../testing/helpers';
import { HelpHint } from './help-hint';

@Component({ imports: [HelpHint], template: `<debrief-help label="Pipeline" text="Steps run in order." />` })
class Host {}

describe('HelpHint', () => {
  it('toggles an explanation linked to the button and closes with Escape', async () => {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    const button = el<HTMLButtonElement>(fixture, 'button');
    expect(button.getAttribute('aria-label')).toBe('Help: Pipeline');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    button.click();
    await settle(fixture);
    const tip = el(fixture, '[role=note]');
    expect(tip.textContent).toBe('Steps run in order.');
    expect(button.getAttribute('aria-controls')).toBe(tip.id);
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=note]')).toBeNull();
  });
});
