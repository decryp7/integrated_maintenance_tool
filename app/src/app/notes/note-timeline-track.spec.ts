import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, provideTinyBackend, settle } from '../../testing/helpers';
import { at } from '../../testing/must';
import { NoteAnchor, NoteItem } from '../core/api.types';
import { NoteActions } from './note-actions';
import { NoteTimelineTrack } from './note-timeline-track';
import { NotesStore } from './notes.store';

const item = (id: string, anchor: NoteAnchor, text: string, severity: NoteItem['severity'] = 'info'): NoteItem => ({
  id,
  anchor,
  text,
  tags: [],
  severity,
  author: 'ana',
  createdMs: 1,
  updatedMs: 1,
  resolved: true,
});

describe('NoteTimelineTrack', () => {
  function render(notes: NoteItem[], view: { from: number; to: number } | null) {
    const items = signal(notes);
    const edit = vi.fn(async () => undefined);
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        { provide: NotesStore, useValue: { items, get: (id: string) => items().find((n) => n.id === id) } },
        { provide: NoteActions, useValue: { edit } },
      ],
    });
    const fixture = TestBed.createComponent(NoteTimelineTrack);
    fixture.componentRef.setInput('view', view);
    fixture.componentRef.setInput('width', 400);
    fixture.detectChanges();
    return { fixture, edit, items };
  }

  const notes = [
    item('moment', { kind: 'instant', at: 100 }, '# Deploy started\nmore'),
    item('outage', { kind: 'range', from: 200, to: 300 }, 'Outage', 'problem'),
    item('event', { kind: 'event', eventId: 1, ts: 150, source: 'a.log', excerpt: 'x' }, 'on a row, not on the timeline'),
    item('later', { kind: 'instant', at: 5000 }, 'outside the view'),
  ];

  it('draws a flag per time note of the view, a line for a moment and a shaded span for a range', async () => {
    const { fixture } = render(notes, { from: 0, to: 400 });
    await settle(fixture);
    const flags = all<HTMLButtonElement>(fixture, '[data-testid=note-mark]');
    expect(flags.map((f) => f.textContent?.trim())).toEqual(['✎ Deploy started', '✎ Outage']);
    expect(at(flags, 0).getAttribute('aria-label')).toMatch(/^Note at .*: Deploy started\. Click to edit\.$/);
    expect(at(flags, 0).style.left).toBe('25%');
    expect(all(fixture, '.line')).toHaveLength(1);
    const span = all<HTMLElement>(fixture, '.span');
    expect(span).toHaveLength(1);
    expect([at(span, 0).style.left, at(span, 0).style.width]).toEqual(['50%', '25%']);
  });

  it('edits the note when its flag is clicked, and ignores a note that is gone', async () => {
    const { fixture, edit, items } = render(notes, { from: 0, to: 400 });
    await settle(fixture);
    at(all<HTMLButtonElement>(fixture, '[data-testid=note-mark]'), 1).click();
    expect(edit).toHaveBeenCalledWith(expect.objectContaining({ id: 'outage' }));
    const flag = at(all<HTMLButtonElement>(fixture, '[data-testid=note-mark]'), 0);
    items.set(notes.filter((n) => n.id !== 'moment'));
    flag.click();
    expect(edit).toHaveBeenCalledTimes(1);
  });

  it('draws nothing without a view', async () => {
    const { fixture } = render(notes, null);
    await settle(fixture);
    expect(all(fixture, '[data-testid=note-mark]')).toHaveLength(0);
  });
});
