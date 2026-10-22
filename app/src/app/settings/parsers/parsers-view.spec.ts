import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, tinyBackend } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { DialogService } from '../../ui/dialog.service';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParserEditor } from './parser-editor';
import { ParsersBackend } from './parsers-backend';
import { ParsersStore } from './parsers.store';
import { ParsersView } from './parsers-view';

async function setup() {
  const debrief = tinyBackend();
  const backend = new MockParsersBackend(debrief);
  TestBed.configureTestingModule({ providers: [provideTinyBackend(debrief), { provide: ParsersBackend, useValue: backend }] });
  await TestBed.inject(WorkspaceStore).init();
  const fixture = TestBed.createComponent(ParsersView);
  await settle(fixture);
  await vi.waitFor(() => expect(all(fixture, 'li.parser').length).toBeGreaterThan(0));
  await settle(fixture);
  return { fixture, store: TestBed.inject(ParsersStore), dialogs: TestBed.inject(DialogService) };
}

describe('ParsersView (I3)', () => {
  it('lists the library beside a detail pane that says what to do until a parser is chosen', async () => {
    const { fixture } = await setup();
    expect(el(fixture, '[data-testid=parsers-detail-hint]').textContent).toContain('Choose Edit');
    expect(fixture.nativeElement.querySelector('debrief-parser-editor')).toBeNull();
  });

  it('shows the editor and its preview inline instead of opening a dialog, and a save puts the library back', async () => {
    const { fixture, store, dialogs } = await setup();
    el<HTMLButtonElement>(fixture, '[aria-label="Edit Serilog CLEF"]').click();
    await vi.waitFor(() => expect(fixture.nativeElement.querySelector('debrief-parser-editor')).not.toBeNull());
    await settle(fixture);
    expect(dialogs.stack().some((d) => d.component === ParserEditor)).toBe(false);
    expect(el(fixture, 'debrief-parser-editor').classList).toContain('embedded');
    await vi.waitFor(() => expect(el(fixture, '[data-testid=parser-preview-summary]').textContent).toContain('events read'));
    expect(store.editing()?.doc.id).toBe('serilog-clef');
    // Cancel leaves the detail pane.
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture);
    expect(store.editing()).toBeNull();
    expect(el(fixture, '[data-testid=parsers-detail-hint]')).toBeTruthy();
  });

  it('opens a blank parser in the detail pane, and closeEditor with a library replaces the list', async () => {
    const { fixture, store } = await setup();
    buttonByText(fixture, 'New blank').click();
    await settle(fixture);
    expect(store.editing()?.isNew).toBe(true);
    expect(fixture.nativeElement.querySelector('debrief-parser-editor')).not.toBeNull();
    store.closeEditor(store.library()!);
    await settle(fixture);
    expect(store.editing()).toBeNull();
    expect(store.notice()).toBe('Saved the parser.');
  });
});
