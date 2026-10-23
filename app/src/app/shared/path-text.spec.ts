import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, settle } from '../../testing/helpers';
import { PathText } from './path-text';

const PATH = 'C:\\ProgramData\\VeryLongVendorName\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log';

@Component({
  imports: [PathText],
  template: `<debrief-path-text [value]="value()" [full]="full()" [copy]="copy()" [keepTail]="keep()" />`,
})
class Host {
  readonly value = signal(PATH);
  readonly full = signal<string | null>(null);
  readonly copy = signal(false);
  readonly keep = signal(32);
}

describe('PathText', () => {
  afterEach(() => vi.restoreAllMocks());

  it('splits a path into a shortenable start and a file name that stays, with the full value as tooltip', async () => {
    const f = TestBed.createComponent(Host);
    f.componentInstance.keep.set(60);
    await settle(f);
    expect(el(f, '.head').textContent).toBe('C:\\ProgramData\\VeryLongVendorName');
    expect(el(f, '.tail').textContent).toBe('\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log');
    expect(el(f, '[data-testid=path-text]').getAttribute('title')).toBe(PATH);
    expect(f.nativeElement.querySelector('button')).toBeNull();
  });

  it('shows one value and offers another (the full path) as tooltip and for copying', async () => {
    const f = TestBed.createComponent(Host);
    f.componentInstance.value.set('a.log');
    f.componentInstance.full.set('/logs/a.log');
    f.componentInstance.copy.set(true);
    await settle(f);
    expect(el(f, '[data-testid=path-text]').getAttribute('title')).toBe('/logs/a.log');
    const write = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText: write } });
    el<HTMLButtonElement>(f, 'button').click();
    await settle(f);
    expect(write).toHaveBeenCalledWith('/logs/a.log');
    expect(el(f, 'button').getAttribute('aria-label')).toBe('Copied');
    vi.unstubAllGlobals();
  });

  it('stays quiet when the clipboard refuses', async () => {
    const f = TestBed.createComponent(Host);
    f.componentInstance.copy.set(true);
    await settle(f);
    vi.stubGlobal('navigator', { clipboard: { writeText: () => Promise.reject(new Error('denied')) } });
    el<HTMLButtonElement>(f, 'button').click();
    await settle(f);
    expect(el(f, 'button').getAttribute('aria-label')).toBe('Copy the full path');
    vi.unstubAllGlobals();
  });
});
