import { TestBed } from '@angular/core/testing';
import { buttonByText, el, all, provideTinyBackend, settle, text, tinyBackend } from '../../../testing/helpers';
import { MockBackend } from '../../core/mock/mock-backend';
import { UxBackend } from '../../core/ux-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { UndoStack } from '../../undo/undo-stack';
import { ProfileDraft } from '../profile-draft';
import { lineDiff, stampToMs } from './line-diff';
import { ProfileHistory } from './profile-history';
import { at } from '../../../testing/must';

describe('lineDiff', () => {
  it('aligns lines with the longest common subsequence after the common head and tail', () => {
    const d = lineDiff('a\nb\nc\nd', 'a\nx\nc\nd\ne');
    expect(d.lines.map((l) => `${l.kind[0]}${l.text}`)).toEqual(['sa', 'rb', 'ax', 'sc', 'sd', 'ae']);
    expect([d.added, d.removed, d.coarse]).toEqual([2, 1, false]);
  });

  it('handles identical, emptied and reordered texts', () => {
    expect(lineDiff('a\r\nb', 'a\nb').added).toBe(0);
    expect(lineDiff('a\nb', '').lines.map((l) => l.kind)).toEqual(['removed', 'removed', 'added']);
    const swap = lineDiff('x\ny', 'y\nx');
    expect(swap.added + swap.removed).toBe(2);
    expect(lineDiff('p\nq', 'q').lines.map((l) => l.kind)).toEqual(['removed', 'same']);
    expect(lineDiff('q', 'p\nq').lines.map((l) => l.kind)).toEqual(['added', 'same']);
  });

  it('falls back to removed-then-added when the changed middle is too large', () => {
    const d = lineDiff('a\nb\nc', 'x\ny\nz', 4);
    expect(d.coarse).toBe(true);
    expect(d.lines.map((l) => l.kind)).toEqual(['removed', 'removed', 'removed', 'added', 'added', 'added']);
  });

  it('reads version stamps as UTC', () => {
    expect(stampToMs('20261017-095244123')).toBe(Date.UTC(2026, 9, 17, 9, 52, 44, 123));
    expect(stampToMs('20261017-095244')).toBe(Date.UTC(2026, 9, 17, 9, 52, 44));
    expect(stampToMs('junk')).toBeNull();
  });
});

describe('ProfileHistory', () => {
  async function render(backend: MockBackend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), ProfileDraft] });
    const draft = TestBed.inject(ProfileDraft);
    await draft.load();
    const fixture = TestBed.createComponent(ProfileHistory);
    await settle(fixture, 6);
    return { fixture, backend, draft };
  }

  async function saveAs(backend: MockBackend, name: string) {
    const toml = (await backend.getProfile()).toml.replace(/^name = .*$/m, `name = "${name}"`);
    TestBed.inject(WorkspaceStore).profileSaved(await backend.saveProfile(toml));
  }

  it('explains the empty history', async () => {
    const { fixture } = await render();
    expect(text(fixture)).toContain('No earlier versions yet: every save keeps the previous one, up to 20.');
  });

  it('lists versions, shows a diff against the current profile and restores one (undoably)', async () => {
    const backend = tinyBackend();
    const { fixture, draft } = await render(backend);
    const original = (await backend.getProfile()).toml;
    await saveAs(backend, 'Second');
    await draft.load();
    await settle(fixture, 6);
    const versions = all<HTMLButtonElement>(fixture, '.ver');
    expect(versions.length).toBe(1);
    expect(text(fixture)).toContain('Select a version');
    at(versions, 0).click();
    await settle(fixture, 6);
    expect(el(fixture, '.summary').textContent).toContain('remove 1 line and add 1');
    expect(all(fixture, '.l.added .sr')[0]?.textContent).toBe('added: ');
    expect(all(fixture, '.l.removed .sign')[0]?.textContent).toBe('−');
    buttonByText(fixture, 'Restore this version').click();
    await settle(fixture, 8);
    expect((await backend.getProfile()).toml).toBe(original);
    expect(draft.doc()?.toml).toBe(original);
    expect(TestBed.inject(WorkspaceStore).notice()).toContain('Restored');
    expect(TestBed.inject(UndoStack).undoLabel()).toBe('profile restore');
    expect(el(fixture, '.summary').textContent).toContain('same as the current profile');
    expect(buttonByText(fixture, 'Restore this version').disabled).toBe(true);
  });

  it('will not restore over unsaved edits, and shows refusals and failures', async () => {
    const backend = tinyBackend();
    const { fixture, draft } = await render(backend);
    await saveAs(backend, 'Second');
    await draft.load();
    await settle(fixture, 6);
    at(all<HTMLButtonElement>(fixture, '.ver'), 0).click();
    await settle(fixture, 6);
    draft.setText(draft.text() + '\n# edit');
    await settle(fixture);
    expect(text(fixture)).toContain('Save or revert your unsaved edits first.');
    expect(buttonByText(fixture, 'Restore this version').disabled).toBe(true);
    draft.revert();
    await settle(fixture);
    const validate = vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'bad', line: 2 });
    buttonByText(fixture, 'Restore this version').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Not restored: line 2: bad');
    validate.mockResolvedValueOnce({ ok: false, error: null, line: null });
    buttonByText(fixture, 'Restore this version').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('the profile is not valid');
    vi.spyOn(backend, 'saveProfile').mockRejectedValueOnce('locked');
    buttonByText(fixture, 'Restore this version').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Not restored: locked');
  });

  it('shows load errors with Retry, unreadable versions and stamps it cannot read', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), ProfileDraft] });
    const ux = TestBed.inject(UxBackend);
    const list = vi.spyOn(ux, 'profileHistory').mockRejectedValueOnce('no folder');
    await TestBed.inject(ProfileDraft).load();
    const fixture = TestBed.createComponent(ProfileHistory);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Could not load the profile history: no folder');
    list.mockResolvedValue([
      { file: 'a', name: 'A', savedAt: 'odd', savedAtMs: null },
      { file: 'b', name: 'B', savedAt: '20261017-095244123', savedAtMs: null },
    ]);
    const version = vi.spyOn(ux, 'profileVersion').mockRejectedValueOnce('gone');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('odd');
    expect(text(fixture)).toContain('2026');
    at(all<HTMLButtonElement>(fixture, '.ver'), 0).click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Could not read this version: gone');
    version.mockResolvedValueOnce({ name: 'A', path: null, toml: 'name = "A"' });
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(el(fixture, '.summary').textContent).toContain('Restoring it would');
    // A version that disappears from the list (pruned) is deselected.
    list.mockResolvedValue([{ file: 'b', name: 'B', savedAt: '20261017-095244123', savedAtMs: 1 }]);
    TestBed.inject(WorkspaceStore).profileSaved((await TestBed.inject(WorkspaceStore).summary())!);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Select a version');
  });
});
