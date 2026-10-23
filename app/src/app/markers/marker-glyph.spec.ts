import { TestBed } from '@angular/core/testing';
import { el, markerKind, provideTinyBackend, settle } from '../../testing/helpers';
import { MarkerGlyph } from './marker-glyph';
import { MarkerStyles, glyphStyleOf, styleOf, summaryIcons } from './marker-styles';
import { WorkspaceStore } from '../core/workspace.store';
import { GlyphStyle } from './markers.types';
import { MarkerKind, WorkspaceSummary } from '../core/api.types';

async function render(inputs: Record<string, unknown>) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  const fixture = TestBed.createComponent(MarkerGlyph);
  for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
  await settle(fixture);
  return fixture;
}

describe('MarkerGlyph', () => {
  it('draws the default shape of built-in kinds, decoratively', async () => {
    const f = await render({ kind: 'error' });
    expect((f.nativeElement as HTMLElement).dataset['glyph']).toBe('shape');
    expect(f.nativeElement.getAttribute('aria-hidden')).toBe('true');
    expect(f.nativeElement.getAttribute('role')).toBeNull();
    expect(f.nativeElement.querySelector('debrief-marker-icon')).not.toBeNull();
  });

  it('draws badges in each shape with contrast-aware text and exposes a label when asked', async () => {
    for (const [shape, tag] of [['circle', 'circle'], ['square', 'rect'], ['diamond', 'polygon'], ['hexagon', 'polygon']] as const) {
      const f = await render({ glyph: { badge: 'DB', shape, color: '#ffe066' } satisfies GlyphStyle, label: 'Database', size: 24 });
      const host = f.nativeElement as HTMLElement;
      expect(host.dataset['glyph']).toBe('badge');
      expect(host.getAttribute('role')).toBe('img');
      expect(host.getAttribute('aria-label')).toBe('Database');
      expect(host.style.width).toBe('24px');
      expect(host.querySelector(tag)).not.toBeNull();
      const text = host.querySelector<HTMLElement>('.badge-text')!;
      expect(text.dataset['t']).toBe('DB');
      expect(text.style.color).toBe('rgb(0, 0, 0)');
      expect(text.style.fontSize).toBe(shape === 'diamond' ? '9.5px' : '11.5px');
      TestBed.resetTestingModule();
    }
    const dark = await render({ glyph: { badge: '1', color: '#102030' } });
    expect((dark.nativeElement as HTMLElement).querySelector<HTMLElement>('.badge-text')!.style.color).toBe('rgb(255, 255, 255)');
  });

  it('draws built-in icons in the marker colour', async () => {
    const f = await render({ glyph: { icon: 'database', color: '#336699' } });
    expect((f.nativeElement as HTMLElement).dataset['glyph']).toBe('icon');
    expect(f.nativeElement.style.color).toBe('rgb(51, 102, 153)');
    expect(f.nativeElement.querySelector('path')!.getAttribute('d')).toContain('M4 6');
  });

  it('renders custom svg only from sanitised elements and custom png as an image', async () => {
    const f = await render({ glyph: { svg: '<svg viewBox="0 0 8 8" onload="x()"><script>alert(1)</script><path d="M0 0L8 8" onclick="y()"/></svg>' } });
    const host = f.nativeElement as HTMLElement;
    expect(host.querySelector('svg path')).not.toBeNull();
    expect(host.innerHTML).not.toMatch(/script|onload|onclick/);
    f.componentRef.setInput('glyph', { icon: 'bug' });
    await settle(f);
    expect(host.querySelector('.slot')).toBeNull();
  });

  it('renders custom png as an image', async () => {
    const p = await render({ glyph: { pngDataUrl: 'data:image/png;base64,AAAA' } });
    expect(el<HTMLImageElement>(p, 'img').getAttribute('src')).toBe('data:image/png;base64,AAAA');
  });

  it('looks kinds up in the workspace summary, including custom icons', async () => {
    const f = await render({ kind: 'fatal' });
    const store = TestBed.inject(WorkspaceStore);
    const summary = store.summary()!;
    expect(summary).not.toBeNull();
    const styles = TestBed.inject(MarkerStyles);
    expect(styles.kind('fatal')?.label).toBeTruthy();
    expect(styles.style('nope')).toEqual({ kind: 'nope' });
    expect(styles.description('nope')).toBeNull();
    expect(styles.description('fatal')).toBeNull();
    expect(f).toBeTruthy();
  });
});

describe('style helpers', () => {
  it('reads the presentation fields the engine sends', () => {
    const k: MarkerKind = markerKind({ kind: 'a', label: 'A', color: '#fff' });
    expect(styleOf(k)).toEqual({ icon: null, badge: null, shape: null, description: null });
    const styled: MarkerKind = { ...k, badge: '2', shape: 'square', icon: 'bug', description: 'd' };
    expect(styleOf(styled)).toEqual({ icon: 'bug', badge: '2', shape: 'square', description: 'd' });
    expect(glyphStyleOf(styled)).toEqual({ kind: 'a', color: '#fff', badge: '2', shape: 'square', icon: 'bug' });
    expect(summaryIcons(null)).toEqual([]);
    expect(summaryIcons(undefined)).toEqual([]);
    expect(summaryIcons({ icons: [{ name: 'x', svg: '<svg/>', pngDataUrl: null }] } as WorkspaceSummary)).toHaveLength(1);
  });
});
