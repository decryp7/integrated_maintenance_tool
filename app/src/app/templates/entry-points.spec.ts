import { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { buttonByText, el, provideFakeDock, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { FormatWizard } from '../format/format-wizard';
import { ProfileEditor } from '../profile/profile-editor';
import { Sidebar } from '../shell/sidebar';
import { READ_ONLY_PROMISE } from '../shared/read-only';
import { DIALOG_REF } from '../ui/dialog.service';
import { TemplatesService } from './templates.service';

describe('template and history entry points', () => {
  it('the profile editor has a History tab, a "New from template" button and a stale-draft notice', async () => {
    const backend = tinyBackend();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const open = vi.spyOn(TestBed.inject(TemplatesService), 'open').mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    const fixture = TestBed.createComponent(ProfileEditor);
    await settle(fixture, 6);
    buttonByText(fixture, 'History').click();
    await settle(fixture, 6);
    expect(el(fixture, 'imt-profile-history')).toBeTruthy();
    expect(buttonByText(fixture, 'History').getAttribute('aria-selected')).toBe('true');
    const load = vi.spyOn((fixture.componentInstance as unknown as { draft: { load(): Promise<void> } }).draft, 'load');
    buttonByText(fixture, 'New from template…').click();
    await settle(fixture);
    expect(open).toHaveBeenCalled();
    expect(load).toHaveBeenCalledTimes(1);
    buttonByText(fixture, 'New from template…').click();
    await settle(fixture);
    expect(load).toHaveBeenCalledTimes(1);

    const draft = (fixture.componentInstance as unknown as { draft: { stale: { set(v: boolean): void }; revert(): void } }).draft;
    draft.stale.set(true);
    await settle(fixture);
    expect(text(fixture)).toContain('The active profile changed (undo or restore)');
    const revert = vi.spyOn(draft, 'revert');
    buttonByText(fixture, 'Discard my edits').click();
    expect(revert).toHaveBeenCalled();
  });

  it('the format wizard offers the gallery for the unrecognised file', async () => {
    const close = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: DIALOG_REF, useValue: { close, closed: new Promise(() => undefined) } }] });
    const open = vi.spyOn(TestBed.inject(TemplatesService), 'open').mockResolvedValue(false);
    const fixture = TestBed.createComponent(FormatWizard as Type<FormatWizard>);
    fixture.componentRef.setInput('path', '/logs/system/export.reg');
    fixture.componentRef.setInput('name', 'export.reg');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=start-from-template]').click();
    expect(close).toHaveBeenCalled();
    expect(open).toHaveBeenCalledWith({ path: '/logs/system/export.reg', name: 'export.reg' });
  });

  it('the source list says that IMT never modifies the files', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock()] });
    const fixture = TestBed.createComponent(Sidebar);
    await TestBed.inject(WorkspaceStore).init();
    await settle(fixture);
    const list = el(fixture, '.sources');
    expect(list.getAttribute('title')).toContain(READ_ONLY_PROMISE);
    expect(el(fixture, '#imt-sources-read-only').textContent).toContain(READ_ONLY_PROMISE);
    expect(list.getAttribute('aria-describedby')).toBe('imt-sources-read-only');
  });
});
